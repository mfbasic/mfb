//! Windows Schannel (SSPI) backend for the `tls::` client helpers. Linked through
//! the IAT (secur32.dll / crypt32.dll); no dlopen bridge. The handshake is a
//! caller-driven `InitializeSecurityContextW` loop over the 47-I socket, followed
//! by `Encrypt`/`DecryptMessage` for the stream. Certificate CHAIN trust is
//! enforced by Schannel during the handshake (`SCH_CRED_AUTO_CRED_VALIDATION`, which
//! fails ISC with e.g. `SEC_E_UNTRUSTED_ROOT`); the HOSTNAME is enforced explicitly
//! after the handshake with `CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL)`
//! against `serverName` — the check the plan flags as easy to omit and never notice.
//!
//! The `tls::` resource record cannot hold Schannel state inline, so
//! `TLS_SCHANNEL_OFFSET_BLOCK` (in the plan-80 record tail) points at an arena
//! STATE block (see `st::*`): the credential/context handles, the negotiated
//! stream sizes, the ciphertext receive buffer, and any leftover decrypted
//! plaintext a short read left behind.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::marshal::*;
use std::collections::HashMap;

use super::{
    TLS_LISTENER_OFFSET_CLOSED, TLS_OFFSET_CLOSED, TLS_OFFSET_FD, TLS_OFFSET_STATE,
    TLS_RECORD_SIZE, TLS_SCHANNEL_OFFSET_BLOCK,
};
use crate::codegen::builtins::net::native::emit_string_result_build;
use crate::target::shared::abi;
const SECUR32: &str = "secur32.dll";
const CRYPT32: &str = "crypt32.dll";

// --- Schannel / SSPI constants (values from sspi.h / schannel.h) ------------
const SECPKG_CRED_OUTBOUND: &str = "2";
const SCHANNEL_CRED_VERSION: &str = "4";
// SCH_CRED_AUTO_CRED_VALIDATION (0x20) | SCH_USE_STRONG_CRYPTO (0x400000).
const SCH_CRED_FLAGS: &str = "4194336";
// ISC_REQ_SEQUENCE_DETECT|REPLAY_DETECT|CONFIDENTIALITY|ALLOCATE_MEMORY|STREAM.
const ISC_REQ_FLAGS: &str = "33052"; // 0x811C
const SECBUFFER_EMPTY: &str = "0";
const SECBUFFER_DATA: &str = "1";
const SECBUFFER_TOKEN: &str = "2";
const SECBUFFER_STREAM_TRAILER: &str = "6";
const SECBUFFER_STREAM_HEADER: &str = "7";
const SECBUFFER_EXTRA: &str = "5";
const SECBUFFER_VERSION: &str = "0";
const SEC_E_OK: &str = "0";
const SECPKG_ATTR_STREAM_SIZES: &str = "4";
const SECPKG_ATTR_REMOTE_CERT_CONTEXT: &str = "83";
// CertVerifyCertificateChainPolicy: CERT_CHAIN_POLICY_SSL = 4.
const CERT_CHAIN_POLICY_SSL: &str = "4";
const USP_NAME: &str = "Microsoft Unified Security Protocol Provider";
// Max TLS record; the ciphertext receive buffer.
const RECV_CAP: usize = 0x4400;

// --- arena STATE block layout ----------------------------------------------
mod st {
    pub const CRED: usize = 0; // CredHandle (16)
    pub const CTXT: usize = 16; // CtxtHandle (16)
    pub const HEADER: usize = 32; // stream header size (u32)
    pub const TRAILER: usize = 36; // stream trailer size (u32)
    pub const MAXMSG: usize = 40; // stream max message (u32)
                                  // 44: server-side marker (u32). Set to 1 by `lower_tls_accept`; 0 on the
                                  // client path (the whole header 0..RECV is zeroed there). Read by
                                  // `lower_tls_close` to skip freeing the listener-owned credential.
    pub const SERVER: usize = 44;
    pub const RECV_LEN: usize = 48; // bytes currently in RECV (ciphertext)
    pub const LEFT_OFF: usize = 56; // read cursor into LEFT plaintext buffer
    pub const LEFT_LEN: usize = 64; // undelivered plaintext bytes in LEFT
                                    // Handshake/close SSPI scratch, kept in the ARENA so every pointer is
                                    // `STATE_reg + const` (an ABSOLUTE address). `sspi_call_ext` computes these as
                                    // `InitializeSecurityContextW`/`AcquireCredentialsHandleW` stack-argument values
                                    // INSIDE its `sub_sp` bracket (DEPTH 1); a stack-frame pointer there would be
                                    // `body_shift` bytes off (finalize_frame only shifts DEPTH-0 accesses), and a
                                    // plain vreg carried across the subtract can spill and reload `body_shift` off —
                                    // exactly what corrupted an ISC output pointer into a NULL return (RIP=0). These
                                    // fields are live only during connect/close, before RECV is ever touched.
    pub const SC_CRED: usize = 72; // SCHANNEL_CRED (0x60)
    pub const OUTBUF: usize = 168; // out SecBuffer[1] (16)
    pub const OUTDESC: usize = 184; // out SecBufferDesc (16)
    pub const INBUF: usize = 200; // in SecBuffer[2] (32)
    pub const INDESC: usize = 232; // in SecBufferDesc (16)
    pub const ATTRS: usize = 248; // context attrs out (u32)
    pub const EXPIRY: usize = 256; // TimeStamp (8)
    pub const RECV: usize = 320; // ciphertext receive buffer (RECV_CAP)
    pub const LEFT: usize = 320 + super::RECV_CAP; // decrypted plaintext buffer
    pub const SIZE: usize = 320 + super::RECV_CAP + super::RECV_CAP;
}

/// Emit `compare(status, SEC_E_INCOMPLETE_MESSAGE); branch_eq(target)`. The status
/// constant is `0x80090318` (negative as i32); the encoder rejects the negative
/// literal, so it is built in `%v14` by shift+add and sign-extended to match the
/// sign-extended status register.
fn branch_if_incomplete(
    status: &str,
    target: &str,
    ins: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v14 = vregs.next();
    ins.extend([
        abi::move_immediate(&v14, "Integer", "32777"), // 0x8009
        abi::shift_left_immediate(&v14, &v14, 16),     // 0x80090000
        abi::add_immediate(&v14, &v14, 792),           // 0x80090318
        abi::sign_extend_word(&v14, &v14),
        abi::compare_registers(status, &v14),
        abi::branch_eq(target),
    ]);
}

fn sym(name: &str) -> String {
    format!("_mfb_tls_w_{}", name.replace([' ', '.'], "_"))
}

fn utf16z_hex(text: &str) -> String {
    let mut hex = String::new();
    for ch in text.chars() {
        let cp = ch as u32;
        hex.push_str(&format!("{:02x}{:02x}", cp & 0xff, (cp >> 8) & 0xff));
    }
    hex.push_str("0000");
    hex
}

fn wide_cstr(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "UTF-16LE string (NUL-terminated)".to_string(),
        align: 2,
        size: (text.len() + 1) * 2,
        value: utf16z_hex(text),
    }
}

/// Read-only wide strings the Schannel helpers reference (the SSPI package name).
/// The server helpers add no data objects (their CryptoAPI calls use integer
/// struct-type selectors and a NULL provider name).
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    vec![wide_cstr(&sym(USP_NAME), USP_NAME)]
}

fn wide_addr(
    from: &str,
    dst: impl Into<Operand>,
    id: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    crate::codegen::memory::arena::emit_data_address(from, dst, &sym(id), ins, rel);
}

/// Emit a Win64 external call: args 0..=3 in `return_register`/`ARG[1..3]`, args
/// 4.. in `ARG[4]`.. spilled to the stack tail above the shadow (bug-384). Sign-
/// extends the return. `lib` is the DLL soname key in `platform_imports`.
#[allow(clippy::too_many_arguments)]
fn sspi_call(
    from: &str,
    symbol: &str,
    _lib: &str,
    n_args: usize,
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if n_args > 4 {
        let stack = n_args - 4;
        let frame = (0x20 + stack * 8 + 15) & !15;
        ins.push(abi::subtract_stack(frame));
        for i in 0..stack {
            ins.push(abi::store_u64(
                abi::c_arg(4 + i),
                abi::stack_pointer(),
                0x20 + i * 8,
            ));
        }
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
        ins.push(abi::add_stack(frame));
    } else {
        platform.emit_libc_call(symbol, from, imports, ins, rel)?;
    }
    ins.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

/// A Win64 external call with more than 4 arguments, extending net's proven
/// `sspi_call` shape past the 8 ABI `ARG` roles.
///
/// Args 0..=3 must ALREADY be in `return_register`/`ARG[1..3]` (caller-set, DEPTH
/// 0 — these are ABI arg roles the remap materializes into rcx/rdx/r8/r9 at the
/// call, exactly like net's `sspi_call`, so they are safe). The stack args (index
/// 4..) are given as `None` (a null → store `xzr`) or `Some(off)` (an ARENA STATE
/// offset → the pointer `state + off`). This helper loads the STATE pointer at
/// DEPTH 0, reserves the Win64 stack tail, and computes each `Some` pointer at
/// DEPTH 1 as `state_reg + off` — valid because `state_reg` holds an ABSOLUTE arena
/// address (`sub_sp` only moves `rsp`), unlike a frame `[sp+off]` which DEPTH 1
/// leaves `body_shift` bytes off (that mismatch produced SECPKG_NOT_FOUND and, for
/// an ISC output pointer, a NULL-return RIP=0). `state_off` is the frame offset of
/// the arena STATE pointer.
#[allow(clippy::too_many_arguments)]
fn sspi_call_ext(
    from: &str,
    symbol: &str,
    state_off: usize,
    stack: &[Option<usize>],
    imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v8 = vregs.next();
    let v9 = vregs.next();
    // No manual sub_sp: the stack args go through outgoing_stack_arg_store, which
    // finalize_frame resolves against the reserved outgoing-args area (Win64 arg 4
    // at [rsp+0x20]) — everything stays at DEPTH 0, so STATE (`%v8`) can spill and
    // reload without the body_shift skew a manual sub_sp bracket introduces (that
    // skew scribbled an output pointer over the socket fd slot).
    ins.push(abi::load_u64(&v8, abi::stack_pointer(), state_off));
    for (i, arg) in stack.iter().enumerate() {
        match arg {
            None => ins.push(abi::outgoing_stack_arg_store(abi::ZERO, i)),
            Some(off) => {
                ins.push(abi::add_immediate(&v9, &v8, *off));
                ins.push(abi::outgoing_stack_arg_store(&v9, i));
            }
        }
    }
    platform.emit_libc_call(symbol, from, imports, ins, rel)?;
    ins.push(abi::sign_extend_word(
        abi::return_register(),
        abi::return_register(),
    ));
    Ok(())
}

include!("schannel_impl.rs");
include!("schannel_server.rs");

#[cfg(test)]
#[path = "schannel_tests.rs"]
mod schannel_tests;
