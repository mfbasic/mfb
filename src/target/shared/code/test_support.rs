//! Shared `#[cfg(test)]` codegen-platform stub for unit tests that lower a
//! single native helper and inspect the emitted instruction stream (e.g. the
//! bug-55 error-path resource-release regression guards).
//!
//! It reports a Linux/AArch64 identity (so the `tls`/`crypto` dispatchers take
//! the OpenSSL path) and lowers every libc/variadic call to a plain `bl` to the
//! named function — enough for the helper to lower and register-allocate. The
//! socket-constant accessors return their Linux values so the `net`/`tls`
//! helpers that consult them lower cleanly. The many file/terminal `emit_*`
//! hooks are `unimplemented!()` because the helpers under test never call them.

use std::collections::HashMap;

use super::{
    AppEntrySpec, CodeFunction, CodeInstruction, CodeRelocation, CodegenPlatform, FsPathOperation,
    ProgramEntrySpec,
};
use crate::target::shared::abi;

/// Minimal Linux/AArch64 codegen platform for lowering-inspection unit tests.
pub(crate) struct TestPlatform;

#[rustfmt::skip]
impl CodegenPlatform for TestPlatform {
    fn target(&self) -> &'static str { "linux_aarch64" }
    fn arch(&self) -> &'static str { "aarch64" }
    fn backend(&self) -> &'static dyn super::mir::Backend { &crate::arch::aarch64::backend::AARCH64_BACKEND }
    fn termios_size(&self) -> usize { 0 }
    fn termios_lflag_offset(&self) -> usize { 0 }
    fn termios_lflag_width(&self) -> usize { 0 }
    fn termios_cc_offset(&self) -> usize { 0 }
    fn termios_echo_flag(&self) -> u64 { 0 }
    fn termios_icanon_flag(&self) -> u64 { 0 }
    fn termios_vmin_index(&self) -> usize { 0 }
    fn termios_vtime_index(&self) -> usize { 0 }
    fn emit_program_exit(&self, _from: &str, _instructions: &mut Vec<CodeInstruction>, _relocations: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_program_exit") }
    fn emit_write(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_write") }
    fn emit_poll_input(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_poll_input") }
    fn emit_is_terminal(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_is_terminal") }
    fn emit_terminal_size(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_terminal_size") }
    fn emit_path_exists(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_path_exists") }
    fn emit_path_stat(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_path_stat") }
    fn stat_mode_offset(&self) -> usize { 0 }
    fn emit_current_directory(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_current_directory") }
    fn emit_environ_pointer(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_environ_pointer") }
    fn emit_fs_path_operation(&self, _from: &str, _op: FsPathOperation, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_fs_path_operation") }
    fn emit_errno(&self, _from: &str, dst: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // Leave a plausible errno value in `dst`; a plain move is enough for
        // helpers (e.g. the non-blocking connect timeout path) to lower and
        // register-allocate.
        instructions.push(abi::move_immediate(dst, "Integer", "0"));
        Ok(())
    }
    fn emit_libc_call(&self, base: &str, _from: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        // A plain `bl` to the named libc function is enough for the helper to
        // lower and register-allocate; the tests inspect the release blocks.
        instructions.push(abi::branch_link(&format!("_{base}")));
        Ok(())
    }
    fn emit_open_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_open_file") }
    fn emit_read_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_read_file") }
    fn emit_close_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_close_file") }
    fn emit_sync_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_sync_file") }
    fn emit_seek_file(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_seek_file") }
    fn emit_rename_path(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_rename_path") }
    fn emit_mkstemps(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_mkstemps") }
    fn emit_random_bytes(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_random_bytes") }
    fn emit_temp_directory(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_temp_directory") }
    fn emit_opendir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_opendir") }
    fn emit_readdir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_readdir") }
    fn emit_closedir(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_closedir") }
    fn dirent_name_offset(&self) -> usize { 0 }
    fn dirent_name_length_offset(&self) -> usize { 0 }
    fn emit_realpath(&self, _from: &str, _pi: &HashMap<String, String>, _i: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> { unimplemented!("TestPlatform::emit_realpath") }
    fn emit_arena_map(&self, _size_reg: &str, _instructions: &mut Vec<CodeInstruction>) -> Result<(), String> { unimplemented!("TestPlatform::emit_arena_map") }
    fn emit_arena_unmap(&self, _instructions: &mut Vec<CodeInstruction>) -> Result<(), String> { unimplemented!("TestPlatform::emit_arena_unmap") }
    fn addrinfo_addr_offset(&self) -> usize { 24 }
    fn sol_socket(&self) -> &'static str { "1" }
    fn so_reuseaddr(&self) -> &'static str { "2" }
    fn so_rcvtimeo(&self) -> &'static str { "20" }
    fn so_sndtimeo(&self) -> &'static str { "21" }
    fn eagain(&self) -> &'static str { "11" }
    fn emsgsize(&self) -> &'static str { "90" }
    fn o_nonblock(&self) -> &'static str { "2048" }
    fn einprogress(&self) -> &'static str { "115" }
    fn so_error(&self) -> &'static str { "4" }
    fn emit_variadic_call(&self, base: &str, _from: &str, _pi: &HashMap<String, String>, instructions: &mut Vec<CodeInstruction>, _r: &mut Vec<CodeRelocation>) -> Result<(), String> {
        instructions.push(abi::branch_link(&format!("_{base}")));
        Ok(())
    }
    fn emit_app_program_entry(&self, _spec: &AppEntrySpec, _pi: &HashMap<String, String>) -> Option<Result<Vec<CodeFunction>, String>> { None }
    fn emit_program_entry(&self, _spec: &ProgramEntrySpec<'_>, _pi: &HashMap<String, String>) -> Result<CodeFunction, String> { unimplemented!("TestPlatform::emit_program_entry") }
    fn emit_thread_trampoline(&self, _pi: &HashMap<String, String>, _uses_stdin: bool, _arena_init: super::ArenaInitSymbols) -> Result<CodeFunction, String> { unimplemented!("TestPlatform::emit_thread_trampoline") }
}

/// Whether a label with `name` appears in the instruction stream.
pub(crate) fn has_label(ins: &[CodeInstruction], name: &str) -> bool {
    use super::CodeOp;
    ins.iter()
        .any(|i| i.op == CodeOp::Label && i.get("name") == Some(name))
}
