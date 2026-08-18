// Regression guard for bug-414 item (2): the Schannel `tls::read`/`tls::readText`
// entry must reject `maxBytes <= 0` with `ErrInvalidArgument`, matching the
// OpenSSL backend (`openssl.rs`, which branches to an `_invalid` exit emitting
// `ERR_INVALID_ARGUMENT`). Before the fix the Schannel read had no such guard:
// `maxBytes == 0` ran a full blocking recv+DecryptMessage then served 0 bytes as
// OK, and a negative `maxBytes` routed to `alloc_fail`/`ErrOutOfMemory` — a
// cross-platform divergence from Linux/macOS. This lowers the read helper and
// pins the presence of the ErrInvalidArgument exit so it cannot silently
// regress. Runtime proof of the Schannel path is Windows-only (box 2230).
// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::app::hook as app;
use crate::codegen::app::hook::*;
use crate::codegen::cleanup::owned::*;
use crate::codegen::cleanup::thread::*;
use crate::codegen::collection::assign::*;
use crate::codegen::collection::buffer::*;
use crate::codegen::collection::compare::*;
use crate::codegen::collection::layout::*;
use crate::codegen::collection::list::*;
use crate::codegen::collection::map::*;
use crate::codegen::collection::search::*;
use crate::codegen::collection::sort::*;
use crate::codegen::compiler::opt::*;
use crate::codegen::engine::analysis::*;
use crate::codegen::engine::arch::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::control::*;
use crate::codegen::engine::convert::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::operators::*;
use crate::codegen::engine::tests::TestPlatform;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::engine::validation;
use crate::codegen::engine::validation::*;
use crate::codegen::engine::value::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::error::result::*;
use crate::codegen::io::stdin::*;
use crate::codegen::io::stdout::*;
use crate::codegen::io::terminal::*;
use crate::codegen::memory::arena::*;
use crate::codegen::memory::data::*;
use crate::codegen::memory::marshal::*;
use crate::codegen::memory::value::*;
use crate::codegen::os::ffi::*;
use crate::codegen::os::process::*;
use crate::codegen::os::syscall::*;
use crate::codegen::resource::cleanup::*;
use crate::codegen::runtime::thread::*;
use crate::codegen::string::format::*;
use crate::codegen::string::repr::*;
use crate::codegen::string::util::*;
use crate::codegen::string::validate::*;
use crate::codegen::term::core as term;
use crate::codegen::term::core::*;
use crate::codegen::term::grid::*;
use std::collections::HashMap;
/// The Schannel read helper emits an `ErrInvalidArgument` failure exit, produced
/// only by `emit_fail(ERR_INVALID_ARGUMENT_*)` — which relocates the error
/// message data symbol. bug-414 (2): before the fix no such exit existed.
fn reads_reject_invalid_maxbytes(text: bool) {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_frame, _ins, rel, _slots) =
        lower_tls_read("t_read", &imports, &TestPlatform, text).expect("lower schannel tls::read");
    let invalid_argument_symbol =
        crate::codegen::registry::runtime_error_emission("ErrInvalidArgument")
            .expect("errorCode name")
            .1;
    assert!(
        rel.iter().any(|r| r.to == invalid_argument_symbol),
        "bug-414: schannel tls::read must reject maxBytes <= 0 with ErrInvalidArgument \
         (text={text})"
    );
}

#[test]
fn read_bytes_rejects_nonpositive_maxbytes() {
    reads_reject_invalid_maxbytes(false);
}

#[test]
fn read_text_rejects_nonpositive_maxbytes() {
    reads_reject_invalid_maxbytes(true);
}
