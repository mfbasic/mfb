// Regression guard for bug-52: on macOS, `tls::readText`'s encoding-error
// exit must release the mapped `dispatch_data` (MAPPED) and the retained nw
// content object (CTX_CONTENT) before failing, exactly as the success exit
// does. Before the fix that exit jumped straight to `emit_fail`, so every
// invalid-UTF-8 read leaked one map + one content object — a peer-controlled
// (remote) memory-exhaustion DoS. Runtime proof lives in the fix's leak
// measurement (`leaks` shows the per-read `dispatch_data_t` leak drop to 0);
// this test pins the codegen so the releases cannot silently regress.
use super::*;
use crate::target::shared::code::mir;

struct TlsReadTestPlatform;

#[rustfmt::skip]
impl CodegenPlatform for TlsReadTestPlatform {
    fn target(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::target") }
    fn arch(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::arch") }
    fn backend(&self) -> &'static dyn crate::target::shared::code::mir::Backend { &crate::arch::aarch64::backend::AARCH64_BACKEND }
    fn termios_size(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_size") }
    fn termios_lflag_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_lflag_offset") }
    fn termios_lflag_width(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_lflag_width") }
    fn termios_cc_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_cc_offset") }
    fn termios_echo_flag(&self) -> u64 { unimplemented!("TlsReadTestPlatform::termios_echo_flag") }
    fn termios_icanon_flag(&self) -> u64 { unimplemented!("TlsReadTestPlatform::termios_icanon_flag") }
    fn termios_vmin_index(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_vmin_index") }
    fn termios_vtime_index(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_vtime_index") }
    fn emit_program_exit(
    &self,
    _from: &str,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_program_exit") }
    fn emit_write(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_write") }
    fn emit_poll_input(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_poll_input") }
    fn emit_is_terminal(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_is_terminal") }
    fn emit_terminal_size(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_terminal_size") }
    fn emit_path_exists(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_exists") }
    fn emit_path_stat(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_stat") }
    fn stat_mode_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::stat_mode_offset") }
    fn emit_current_directory(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_current_directory") }
    fn emit_environ_pointer(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_environ_pointer") }
    fn emit_fs_path_operation(
    &self,
    _from: &str,
    _operation: crate::target::shared::code::FsPathOperation,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_fs_path_operation") }
    fn emit_errno(
    &self,
    _from: &str,
    _dst: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_errno") }
    fn emit_libc_call(
    &self,
    _base: &str,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
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
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_open_file") }
    fn emit_read_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_read_file") }
    fn emit_close_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_close_file") }
    fn emit_sync_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_sync_file") }
    fn emit_seek_file(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_seek_file") }
    fn emit_rename_path(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_rename_path") }
    fn emit_mkstemps(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_mkstemps") }
    fn emit_random_bytes(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_random_bytes") }
    fn emit_temp_directory(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_temp_directory") }
    fn emit_opendir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_opendir") }
    fn emit_readdir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_readdir") }
    fn emit_closedir(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_closedir") }
    fn dirent_name_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::dirent_name_offset") }
    fn dirent_name_length_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::dirent_name_length_offset") }
    fn emit_realpath(
    &self,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_realpath") }
    fn emit_arena_map(
    &self,
    _size_reg: &str,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_map") }
    fn emit_arena_unmap(&self, _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_unmap") }
    fn addrinfo_addr_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::addrinfo_addr_offset") }
    fn sol_socket(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::sol_socket") }
    fn so_reuseaddr(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_reuseaddr") }
    fn so_rcvtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_rcvtimeo") }
    fn so_sndtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_sndtimeo") }
    fn eagain(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::eagain") }
    fn emsgsize(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::emsgsize") }
    fn o_nonblock(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::o_nonblock") }
    fn einprogress(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::einprogress") }
    fn so_error(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_error") }
    fn emit_variadic_call(
    &self,
    _base: &str,
    _from: &str,
    _platform_imports: &HashMap<String, String>,
    _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_variadic_call") }
    fn emit_program_entry(
    &self,
    _spec: &crate::target::shared::code::ProgramEntrySpec<'_>,
    _platform_imports: &HashMap<String, String>,
) -> Result<crate::target::shared::code::CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_program_entry") }
    fn emit_thread_trampoline(
    &self,
    _platform_imports: &HashMap<String, String>,
    _uses_stdin: bool,
    _arena_init: crate::target::shared::code::ArenaInitSymbols,
) -> Result<crate::target::shared::code::CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_thread_trampoline") }
}

/// Number of `blr` (indirect call) instructions between the `start` and the
/// next `end` label in the finalized instruction stream.
fn blr_between(ins: &[CodeInstruction], start: &str, end: &str) -> usize {
    let s = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name") == Some(start))
        .unwrap_or_else(|| panic!("missing label {start}"));
    let e = ins[s + 1..]
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name") == Some(end))
        .map(|p| p + s + 1)
        .unwrap_or_else(|| panic!("missing label {end}"));
    ins[s + 1..e]
        .iter()
        .filter(|i| i.op == CodeOp::BranchLinkRegister)
        .count()
}

#[test]
fn readtext_encoding_error_releases_mapped_and_content() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_frame, ins, rel, _slots) =
        lower_tls_read_macos("t_readtext", &imports, &TlsReadTestPlatform, true)
            .expect("lower tls::readText");

    // The encoding-error exit performs exactly the two dispatch_release
    // calls the success path does (MAPPED, then CTX_CONTENT) before failing.
    let releases = blr_between(&ins, "t_readtext_encoding_error", "t_readtext_peer_closed");
    assert_eq!(
        releases, 2,
        "bug-52: encoding_error exit must release MAPPED and CTX_CONTENT before failing"
    );

    // The fix adds a second dlsym(dispatch_release); the whole helper now
    // resolves that data symbol on both the success and the error path
    // (each resolution emits a hi/lo relocation pair).
    let release_relocs = rel
        .iter()
        .filter(|r| r.to.contains("dispatch_release"))
        .count();
    assert!(
        release_relocs >= 4,
        "expected dispatch_release resolved on both exits, got {release_relocs}"
    );
}

#[test]
fn readbytes_has_no_encoding_error_exit() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_frame, ins, _rel, _slots) =
        lower_tls_read_macos("t_readbytes", &imports, &TlsReadTestPlatform, false)
            .expect("lower tls::read");

    // readBytes has no UTF-8 validation, so it never emits an encoding_error
    // label — confirming the bug-52 fix is scoped to the text path only.
    assert!(
        !ins.iter()
            .any(|i| i.op == CodeOp::Label && i.get("name") == Some("t_readbytes_encoding_error")),
        "tls::read (bytes) must not have an encoding_error exit"
    );
}

fn has_label(ins: &[CodeInstruction], name: &str) -> bool {
    ins.iter()
        .any(|i| i.op == CodeOp::Label && i.get("name") == Some(name))
}

/// The instructions from label `start` up to (not including) label `end`.
fn window<'a>(ins: &'a [CodeInstruction], start: &str, end: &str) -> &'a [CodeInstruction] {
    let at = |name: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name") == Some(name))
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
    win.iter().any(|i| i.get("symbol") == Some(&want))
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
    let (_f, ins, _r, _s) =
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
    let (_f, ins, _r, _s) =
        lower_tls_connect_macos("t_c", &imports, &TlsReadTestPlatform).expect("lower");
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
#[test]
fn readtext_releases_previous_semaphore() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_f, ins, rel, _s) =
        lower_tls_read_macos("t_rt", &imports, &TlsReadTestPlatform, true).expect("lower");
    assert!(
        has_label(&ins, "t_rt_sem_skip_release"),
        "readText must release the prior semaphore before creating a fresh one"
    );
    assert!(
        rel.iter().any(|r| r.to.contains("dispatch_release")),
        "readText must resolve dispatch_release for the semaphore free"
    );
}

#[test]
fn write_releases_previous_semaphore() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_f, ins, _r, _s) =
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
    let (_f, _ins, rel, _s) =
        lower_tls_connect_macos("t_c", &imports, &TlsReadTestPlatform).expect("lower");
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
    let (_f, ins, rel, _s) =
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
    let (_f, ins, _r, _s) =
        lower_tls_accept_macos("t_a", &imports, &TlsReadTestPlatform).expect("lower");
    // The accepted-record build stores x31 (zero) into REC_QUEUE rather than
    // the shared listener queue; assert no `store [x1+REC_QUEUE] <- vN` from a
    // loaded queue exists by checking the record store uses the zero register.
    let stores_zero_queue = ins.iter().any(|i| {
        i.op == CodeOp::StrU64
            && i.get("src") == Some(abi::ZERO)
            && i.get("base") == Some(abi::RET[1])
            && i.get("offset") == Some(&REC_QUEUE.to_string())
    });
    assert!(
        stores_zero_queue,
        "accept must store 0 in the accepted socket's queue slot (shared listener queue)"
    );
}

// bug-55: closeListener releases the listener, its queue, and the listener
// ctx semaphore; before the fix it only cancelled the listener.
#[test]
fn close_listener_releases_queue_and_sem() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_f, _ins, rel, _s) =
        lower_tls_close_listener_macos("t_ll", &imports, &TlsReadTestPlatform).expect("lower");
    assert!(
        rel.iter().any(|r| r.to.contains("dispatch_release")),
        "closeListener must resolve dispatch_release for the queue and ctx semaphore"
    );
}
