//! OpenSSL `dlopen`/`dlsym` TLS backend: the socket-timeout connect/read/
//! write/close helpers and their OpenSSL machinery (see `super` for the
//! shared emit helpers and `macos` for the Network.framework backend).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use std::collections::HashMap;

use super::gen_shared::*;
// plan-176-T: `connect.rs` names `super::gen_shared::connect_arg_prologue`, written when `super` was `tls`.
use super::gen_shared;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::emit_data_address;
// bug-575: `tls::connect`/`tls::listen` marshal their host name and PEM paths
// into arena C-strings for OpenSSL; those blocks are helper scratch and are
// released at the single `ret`.
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::marshal::push_write_payload_view;
use crate::codegen::os::socket::shared::{emit_accept_call, emit_socket_type_cloexec};
use crate::target::shared::abi;

mod accept;
mod close;
mod connect;
mod io;
mod listen;

pub(crate) use accept::lower_tls_accept_openssl;
pub(crate) use close::{lower_tls_close_listener_openssl, lower_tls_close_openssl};
pub(crate) use connect::lower_tls_connect_openssl;
pub(crate) use io::{lower_tls_poll_openssl, lower_tls_read_openssl, lower_tls_write_openssl};
pub(crate) use listen::lower_tls_listen_openssl;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod error_path_release_tests;
