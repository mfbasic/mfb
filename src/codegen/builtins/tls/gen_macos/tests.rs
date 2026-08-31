// Regression guard for bug-52: on macOS, `tls::read`'s encoding-error
// exit must release the mapped `dispatch_data` (MAPPED) and the retained nw
// content object (CTX_CONTENT) before failing, exactly as the success exit
// does. Before the fix that exit jumped straight to `emit_fail`, so every
// invalid-UTF-8 read leaked one map + one content object — a peer-controlled
// (remote) memory-exhaustion DoS. Runtime proof lives in the fix's leak
// measurement (`leaks` shows the per-read `dispatch_data_t` leak drop to 0);
// this test pins the codegen so the releases cannot silently regress.
// --- codegen tier imports (migration) ---
use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use std::collections::HashMap;
struct TlsReadTestPlatform;

#[rustfmt::skip]
impl CodegenPlatform for TlsReadTestPlatform {
    fn target(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::target") }
    fn arch(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::arch") }
    fn backend(&self) -> &'static dyn crate::codegen::engine::mir::Backend { &crate::arch::aarch64::backend::AARCH64_BACKEND }
    fn emit_apply_raw_mode(&self, _b: &str, _o: usize, _m: usize, _de: bool, _dc: bool, _i: &mut Vec<CodeInstruction>) { unimplemented!("TlsReadTestPlatform::emit_apply_raw_mode") }
    fn emit_program_exit(
    &self,
    _from: &str,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_program_exit") }
    fn emit_write(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_write") }
    fn emit_poll_input(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_poll_input") }
    fn emit_is_terminal(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_is_terminal") }
    fn emit_terminal_size(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_terminal_size") }
    fn emit_path_exists(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_exists") }
    fn emit_path_stat(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_stat") }
    fn emit_stat_is_kind(&self, _so: usize, _ek: &str, _m: &str, _mk: &str, _e: &str, _f: &str, _mi: &str, _i: &mut Vec<CodeInstruction>) { unimplemented!("TlsReadTestPlatform::emit_stat_is_kind") }
    fn emit_current_directory(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_current_directory") }
    fn emit_environ_pointer(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_environ_pointer") }
    fn emit_fs_path_operation(
    &self,
    _from: &str,
    _operation: FsPathOperation,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_fs_path_operation") }
    fn emit_errno(
    &self,
    _from: &str,
    _dst: Operand,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_errno") }
    fn emit_external_call(
    &self,
    _base: &str,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
        // Minimal stand-in: a plain `bl` to the named libc function is
        // enough for the read helper to lower and register-allocate; the
        // test only inspects the resulting encoding-error release block.
        _instructions.push(crate::target::shared::abi::branch_link(&format!("_{_base}")));
        Ok(())
    }
    fn emit_open_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
        // Same minimal stand-in as `emit_external_call`: `tls::listen` reads the
        // cert and key PEMs through these, and the tests below inspect its frame
        // setup, not the file calls themselves.
        _instructions.push(crate::target::shared::abi::branch_link("_emit_open_file"));
        Ok(())
    }
    fn emit_read_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
        // Same minimal stand-in as `emit_external_call`: `tls::listen` reads the
        // cert and key PEMs through these, and the tests below inspect its frame
        // setup, not the file calls themselves.
        _instructions.push(crate::target::shared::abi::branch_link("_emit_read_file"));
        Ok(())
    }
    fn emit_close_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
        // Same minimal stand-in as `emit_external_call`: `tls::listen` reads the
        // cert and key PEMs through these, and the tests below inspect its frame
        // setup, not the file calls themselves.
        _instructions.push(crate::target::shared::abi::branch_link("_emit_close_file"));
        Ok(())
    }
    fn emit_sync_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_sync_file") }
    fn emit_seek_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
        _instructions.push(crate::target::shared::abi::branch_link("_emit_seek_file"));
        Ok(())
    }
    fn emit_rename_path(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_rename_path") }
    fn emit_mkstemps(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_mkstemps") }
    fn emit_random_bytes(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_random_bytes") }
    fn emit_temp_directory(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_temp_directory") }
    fn emit_opendir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_opendir") }
    fn emit_readdir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_readdir") }
    fn emit_closedir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_closedir") }
    fn emit_read_dir_entry(&self, _p: &str, _np: &str, _nl: &str, _b: &str, _s: &str, _i: &mut Vec<CodeInstruction>) { unimplemented!("TlsReadTestPlatform::emit_read_dir_entry") }
    fn emit_realpath(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_realpath") }
    fn emit_arena_map(
    &self,
    _size_reg: &str,
    _instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_map") }
    fn emit_arena_unmap(&self, _instructions: &mut Vec<CodeInstruction>) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_unmap") }
    fn addrinfo_addr_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::addrinfo_addr_offset") }
    fn sol_socket(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::sol_socket") }
    fn so_reuseaddr(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_reuseaddr") }
    fn so_rcvtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_rcvtimeo") }
    fn so_sndtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_sndtimeo") }
    // plan-110-A net::ping constants: this platform drives only the TLS read path
    // and never reaches them, so keep the stub's fail-loudly convention.
    fn so_rcvbuf(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_rcvbuf") }
    fn ipproto_ip(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::ipproto_ip") }
    fn ip_ttl(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::ip_ttl") }
    fn ip_recvttl(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::ip_recvttl") }
    fn cmsg_ip_ttl_type(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::cmsg_ip_ttl_type") }
    fn clock_monotonic(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::clock_monotonic") }
    fn socket_would_block_code(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::socket_would_block_code") }
    fn socket_message_size_code(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::socket_message_size_code") }
    fn socket_in_progress_code(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::socket_in_progress_code") }
    fn emit_set_nonblocking(&self, _fd: usize, _fl: usize, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_set_nonblocking") }
    fn so_error(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_error") }
    fn emit_variadic_external_call(
    &self,
    _base: &str,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_variadic_external_call") }
    fn emit_program_entry(
    &self,
    _spec: &ProgramEntrySpec<'_>,
    _platform_imports: &HashMap<String, String>,
) -> Result<CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_program_entry") }
    fn emit_thread_trampoline(
    &self,
    _platform_imports: &HashMap<String, String>,
    _uses_stdin: bool,
    _arena_init: ArenaInitSymbols,
) -> Result<CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_thread_trampoline") }
}

/// Number of `blr` (indirect call) instructions between the `start` and the
/// next `end` label in the finalized instruction stream.
fn blr_between(ins: &[CodeInstruction], start: &str, end: &str) -> usize {
    let s = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(start))
        .unwrap_or_else(|| panic!("missing label {start}"));
    let e = ins[s + 1..]
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(end))
        .map(|p| p + s + 1)
        .unwrap_or_else(|| panic!("missing label {end}"));
    ins[s + 1..e]
        .iter()
        .filter(|i| i.op == CodeOp::BranchLinkRegister)
        .count()
}

// bug-52 was a leak on `tls::readText`'s encoding-error exit: it failed without
// releasing the `dispatch_data` map and the retained nw content, so a peer that
// kept sending invalid UTF-8 to a program looping on readText drove an unbounded
// leak — a remotely-triggerable memory-exhaustion DoS.
//
// plan-110-D deleted `tls::readText`, so that exit no longer exists and the
// original assertions have no subject. The PROPERTY it protected still matters
// and now lives one level down: `tls::read`'s drain is the only place that maps
// an nw content object, and it must release both the map and the content before
// publishing the plaintext into CTX_PEND. If it did not, the same peer-driven
// leak would be back with a different label on it.
#[test]
fn read_drain_releases_mapped_and_content_before_publishing() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, rel, _slots) =
        lower_tls_read_macos("t_read", &imports, &TlsReadTestPlatform).expect("lower tls::read");

    // Exactly two indirect calls between publishing the copy and serving it:
    // dispatch_release(MAPPED) and dispatch_release(ctx->pcontent).
    let releases = blr_between(&ins, "t_read_drain_publish", "t_read_check_pend");
    assert_eq!(
        releases, 2,
        "bug-52: the drain must release the map and the retained content before \
         publishing into CTX_PEND, or a peer can drive an unbounded leak"
    );
    assert!(
        rel.iter()
            .filter(|r| r.to.contains("dispatch_release"))
            .count()
            >= 2,
        "the drain must resolve dispatch_release"
    );
}

// plan-110-D: `tls::read` is bytes-only, so no path validates UTF-8 and no
// encoding-error exit is emitted at all. This is the successor to the old
// `readbytes_has_no_encoding_error_exit`, which distinguished the byte form from
// a text form that no longer exists.
#[test]
fn read_has_no_encoding_error_exit() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) = lower_tls_read_macos("t_readbytes", &imports, &TlsReadTestPlatform)
        .expect("lower tls::read");
    assert!(
        !ins.iter().any(|i| i.op == CodeOp::Label
            && i.get("name").as_deref() == Some("t_readbytes_encoding_error")),
        "tls::read must not have an encoding_error exit"
    );
}

// plan-110-D: a read bounded by `tls::setReadTimeout` must leave the outstanding
// receive ARMED when its deadline elapses. `nw_connection_receive` cannot be
// cancelled, so the completion block will still land in CTX_PCONTENT later; if
// the timeout exit cleared CTX_ARMED, the next read would post a SECOND receive
// and the first one's bytes would be stranded (and its content object leaked).
#[test]
fn read_timeout_exit_leaves_the_receive_armed() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) =
        lower_tls_read_macos("t_rto", &imports, &TlsReadTestPlatform).expect("lower tls::read");
    let win = window(&ins, "t_rto_read_timeout", "t_rto_load_fail");
    assert!(
        !win.iter().any(|i| {
            i.op == CodeOp::StrU64 && i.get("offset").as_deref() == Some(&CTX_ARMED.to_string())
        }),
        "the read-timeout exit must not write CTX_ARMED: the receive is still outstanding"
    );
    assert!(
        !win.iter().any(|i| i.op == CodeOp::BranchLinkRegister),
        "the read-timeout exit must not release anything: nw still owns the content"
    );
}

fn has_label(ins: &[CodeInstruction], name: &str) -> bool {
    ins.iter()
        .any(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
}

/// The instructions from label `start` up to (not including) label `end`.
fn window<'a>(ins: &'a [CodeInstruction], start: &str, end: &str) -> &'a [CodeInstruction] {
    let at = |name: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing label {name}"))
    };
    let (from, to) = (at(start), at(end));
    assert!(from < to, "expected {start} to precede {end}");
    &ins[from..to]
}

/// Whether `dlsym(<name>)` is emitted inside this instruction window.
///
/// `emit_dlsym` materialises the symbol's data address with an `adrp`
/// carrying `_mfb_tls_sym_<name>`, so the resolution is visible positionally
/// in the instruction stream. A whole-function relocation scan cannot
/// substitute here: `accept` already resolves `nw_release` in its listener
/// drain loop, so only a windowed check proves the *error exits* release.
fn resolves_in(win: &[CodeInstruction], name: &str) -> bool {
    let want = sym_data_symbol(name);
    win.iter()
        .any(|i| i.get("symbol").as_deref() == Some(&want))
}

// bug-317 T1: `accept` owns a +1 on the popped connection (the
// new-connection trampoline retains it into the ring). Its handshake-failure
// exits used to only `nw_connection_cancel`, which stops network activity
// but keeps the retain — so a server looping on `tls::accept` leaked one
// nw_connection per handshake failure, an unbounded remote-triggerable DoS.
#[test]
fn accept_failure_exits_release_the_connection() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_accept_macos("t_a", &imports, &TlsReadTestPlatform).expect("lower");
    // Each exit is checked against its own window (up to the next exit's
    // label), so one exit's release cannot stand in for the other's.
    for (exit, end) in [
        ("t_a_conn_fail", "t_a_hs_timeout"),
        ("t_a_hs_timeout", "t_a_accept_timeout"),
    ] {
        let win = window(&ins, exit, end);
        assert!(
            resolves_in(win, "nw_connection_cancel"),
            "{exit} must cancel the accepted connection"
        );
        assert!(
            resolves_in(win, "nw_release"),
            "{exit} must nw_release the accepted connection, not just cancel it"
        );
        // The accepted socket shares the listener's serial queue, so these
        // exits must NOT release it — that would over-release a queue still
        // in use by the listener and every other accepted socket.
        assert!(
            !resolves_in(win, "dispatch_release"),
            "{exit} must not release the shared listener queue"
        );
    }
}

// bug-317 T3: `connect`'s failure exits own both the nw_connection (+1 from
// nw_connection_create) and the per-connection dispatch queue; the success
// path hands both to the record for `close` to release. Cancelling alone
// leaked one connection and one queue per failed connect.
#[test]
fn connect_failure_exits_release_connection_and_queue() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_connect_macos("t_c", &imports, &TlsReadTestPlatform, false).expect("lower");
    for (exit, end) in [
        ("t_c_conn_fail", "t_c_conn_timeout"),
        ("t_c_conn_timeout", "t_c_net_fail"),
    ] {
        let win = window(&ins, exit, end);
        assert!(
            resolves_in(win, "nw_connection_cancel"),
            "{exit} must cancel the connection"
        );
        assert!(
            resolves_in(win, "nw_release"),
            "{exit} must nw_release the connection, not just cancel it"
        );
        assert!(
            resolves_in(win, "dispatch_release"),
            "{exit} must dispatch_release the per-connection queue"
        );
    }
}

// bug-55: `emit_fresh_sem` used to store a brand-new dispatch_semaphore into
// ctx->sem on every readText/write, leaking the previous one (~211k residual
// objects over 200k reads under `leaks`). The fix releases the prior
// semaphore first, emitting a `<sym>_sem_skip_release` guard label. These
// tests pin that label so the release cannot silently regress.
// plan-110-D: `tls::read` no longer recycles CTX_SEM -- it waits on CTX_PSEM,
// created once at connection setup. Recycling CTX_SEM underneath a `tls::write`
// that has a send outstanding is the stale-semaphore hazard the fresh-sem
// invariant exists to prevent, so read must not do it. (`tls::write` still does,
// and `write_releases_previous_semaphore` below pins that.)
#[test]
fn read_does_not_recycle_the_shared_semaphore() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _s) =
        lower_tls_read_macos("t_rd", &imports, &TlsReadTestPlatform).expect("lower");
    assert!(
        !has_label(&ins, "t_rd_sem_skip_release"),
        "tls::read must not recycle CTX_SEM: tls::write may have a send outstanding on it"
    );
}

#[test]
fn write_releases_previous_semaphore() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_write_macos("t_w", &imports, &TlsReadTestPlatform, false).expect("lower");
    assert!(
        has_label(&ins, "t_w_sem_skip_release"),
        "write must release the prior semaphore before creating a fresh one"
    );
}

// bug-55: connect retains the endpoint/parameters via nw_connection_create,
// so it must nw_release its own references; before the fix they leaked on
// every successful connect.
#[test]
fn connect_releases_endpoint_and_params() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_ins, rel, _s) =
        lower_tls_connect_macos("t_c", &imports, &TlsReadTestPlatform, false).expect("lower");
    assert!(
        rel.iter().any(|r| r.to.contains("nw_release")),
        "connect must resolve nw_release to free the endpoint and parameters"
    );
}

// bug-55: close now releases the connection (nw_release) and — only when it
// owns them — the dispatch queue and ctx semaphore. The queue release is
// guarded by a `<sym>_skip_queue_release` label because an accepted socket
// shares the listener's queue (queue slot = 0) and must not release it.
#[test]
fn close_releases_connection_queue_and_sem() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, rel, _s) =
        lower_tls_close_macos("t_cl", &imports, &TlsReadTestPlatform).expect("lower");
    assert!(
        rel.iter().any(|r| r.to.contains("nw_release")),
        "close must resolve nw_release for the connection"
    );
    assert!(
        rel.iter().any(|r| r.to.contains("dispatch_release")),
        "close must resolve dispatch_release for the queue and semaphore"
    );
    assert!(
        has_label(&ins, "t_cl_skip_queue_release"),
        "close must guard the queue release so an accepted (queue=0) socket skips it"
    );
}

// bug-55: an accepted socket stores 0 in its queue slot (it shares the
// listener's serial queue), so the shared close skips the queue release.
#[test]
fn accept_stores_zero_queue_slot() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_accept_macos("t_a", &imports, &TlsReadTestPlatform).expect("lower");
    // The accepted-record build stores x31 (zero) into REC_QUEUE rather than
    // the shared listener queue; assert no `store [x1+REC_QUEUE] <- vN` from a
    // loaded queue exists by checking the record store uses the zero register.
    let stores_zero_queue = ins.iter().any(|i| {
        i.op == CodeOp::StrU64
            && i.get("src").as_deref() == Some(abi::ZERO)
            && i.get("base").as_deref() == Some(abi::mfb_return(1).render().as_str())
            && i.get("offset").as_deref() == Some(&REC_QUEUE.to_string())
    });
    assert!(
        stores_zero_queue,
        "accept must store 0 in the accepted socket's queue slot (shared listener queue)"
    );
}

/// bug-412: a cancel-drain loop that blocks on the arena ctx's semaphore
/// (offset `CTX_SEM`) until the terminal `cancelled` state is observed at
/// `CTX_STATE`, so no queued state-changed handler can dereference the freed
/// arena ctx after the exit returns and the program tears the arena down.
/// Mirrors the connect-path drain (bug-380, `client.rs`). Detected
/// structurally within `win`: the loop's back-edge label, an
/// `ldr_u32 [ctx+CTX_STATE]`, a `cmp_imm <cancelled_state>`, and a
/// `b.ne <drain_label>` that closes the loop.
fn has_cancel_drain(win: &[CodeInstruction], drain_label: &str, cancelled_state: &str) -> bool {
    let has_drain_label = win
        .iter()
        .any(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(drain_label));
    let reads_state = win.iter().any(|i| {
        i.op == CodeOp::LdrU32 && i.get("offset").as_deref() == Some(&CTX_STATE.to_string())
    });
    let checks_cancelled = win
        .iter()
        .any(|i| i.op == CodeOp::CmpImm && i.get("rhs").as_deref() == Some(cancelled_state));
    let loops_back = win
        .iter()
        .any(|i| i.op == CodeOp::BranchNe && i.get("target").as_deref() == Some(drain_label));
    has_drain_label && reads_state && checks_cancelled && loops_back
}

// bug-412: `accept`'s handshake-failure exits cancel the accepted connection
// (`nw_connection_cancel` is asynchronous) and used to return immediately. The
// per-connection state handler runs over the arena-allocated CCTX on the
// listener's shared serial queue; if the server exits before the async
// `cancelled` transition fires, that pending handler dereferences the freed
// CCTX (EXC_BAD_ACCESS). Each fail exit must drain to `cancelled` (connection
// state 5) before returning, mirroring the connect-path drain bug-380 added.
#[test]
fn accept_failure_exits_drain_to_cancelled() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_accept_macos("t_a", &imports, &TlsReadTestPlatform).expect("lower");
    for (exit, end, drain) in [
        ("t_a_conn_fail", "t_a_hs_timeout", "t_a_conn_fail_drain"),
        (
            "t_a_hs_timeout",
            "t_a_accept_timeout",
            "t_a_hs_timeout_drain",
        ),
    ] {
        let win = window(&ins, exit, end);
        assert!(
            has_cancel_drain(win, drain, "5"),
            "{exit} must drain to the connection `cancelled` state (5) before failing, \
             so a queued state handler cannot run against the freed CCTX"
        );
    }
}

// bug-412: `closeListener` cancels the listener (`nw_listener_cancel` is
// asynchronous) and used to return immediately. The listener's state handler
// runs over the arena-allocated LCTX and still fires the `cancelled`
// transition; a process exit before it runs dereferences the freed LCTX.
// closeListener must drain to the listener `cancelled` state (listener state 4)
// before returning.
#[test]
fn close_listener_drains_to_cancelled() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, rel, _s) =
        lower_tls_close_listener_macos("t_ll", &imports, &TlsReadTestPlatform).expect("lower");
    assert!(
        has_cancel_drain(&ins, "t_ll_lcancel_drain", "4"),
        "closeListener must drain to the listener `cancelled` state (4) before returning, \
         so a queued listener state handler cannot run against the freed LCTX"
    );
    assert!(
        rel.iter().any(|r| r.to.contains("dispatch_semaphore_wait")),
        "the closeListener drain must resolve dispatch_semaphore_wait to block on LCTX->sem"
    );
}

// bug-55: closeListener releases the listener, its queue, and the listener
// ctx semaphore; before the fix it only cancelled the listener.
#[test]
fn close_listener_releases_queue_and_sem() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_ins, rel, _s) =
        lower_tls_close_listener_macos("t_ll", &imports, &TlsReadTestPlatform).expect("lower");
    assert!(
        rel.iter().any(|r| r.to.contains("dispatch_release")),
        "closeListener must resolve dispatch_release for the queue and ctx semaphore"
    );
}

// bug-462: `tls::listen`'s CoreFoundation cleanup slots were never initialized.
//
// The `cert_fail` exit best-effort-releases CERTREF/KEYREF/ITEMS/DATA, each
// NULL-guarded, and its comment asserted that an exit taken before a slot was
// filled would find it "still NULL -- a no-op". Nothing established that: the
// prologue stored only the four arguments, so the four cleanup slots held
// whatever the caller had left on the stack. Any failure BEFORE both refs were
// set -- an unreadable cert, a malformed PEM, an encrypted key, a mismatched
// pair -- therefore `CFRelease`d stack garbage.
//
// Measured on macOS aarch64 before the fix: `tls::listen` with a
// passphrase-protected key died with `EXC_BREAKPOINT` in
// `CF_IS_OBJC <- CFRelease`, exit 133, instead of raising a catchable
// `ErrTlsFailed`. That is a server's misconfiguration path, so it is the one an
// operator is most likely to take.
#[test]
fn listen_zeroes_its_cleanup_slots_before_any_failure_exit() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) = lower_tls_listen_macos("t_listen", &imports, &TlsReadTestPlatform)
        .expect("lower macos tls::listen");

    // The frame offsets the cert_fail exit releases (server.rs DATA/ITEMS/
    // CERTREF/KEYREF). Hardcoded on purpose: this test exists to notice if a slot
    // is added to that release list without being zeroed here.
    const CLEANUP_SLOTS: [&str; 4] = ["168", "176", "184", "192"];

    // Everything up to the first branch that can reach `cert_fail`.
    let first_fail = ins
        .iter()
        .position(|i| {
            i.get("target").as_deref() == Some("t_listen_cert_fail")
                || i.get("target").as_deref() == Some("t_listen_read_fail_fd")
        })
        .expect("listen must have a failure branch");

    for slot in CLEANUP_SLOTS {
        let zeroed = ins[..first_fail].iter().any(|i| {
            i.get("offset").as_deref() == Some(slot)
                && i.get("src").as_deref() == Some(abi::ZERO)
                && i.get("base").as_deref() == Some(abi::stack_pointer())
        });
        assert!(
            zeroed,
            "bug-462: cleanup slot +{slot} must be zeroed before any exit that \
             CFReleases it, or the exit releases stack garbage"
        );
    }
}
