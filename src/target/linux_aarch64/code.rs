use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{
    self, CodeInstruction, CodeRelocation, NativeCodePlan, ARENA_ALLOC_SYMBOL,
};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
) -> Result<NativeCodePlan, String> {
    code::lower_module_for_platform(module, native_plan, &Platform)
}

struct Platform;

const LINUX_PROT_READ_WRITE: &str = "3";
const LINUX_MAP_PRIVATE_ANON: &str = "34";
const LINUX_SYSCALL_MMAP: &str = "222";
const LINUX_SYSCALL_MUNMAP: &str = "215";
const LINUX_SYSCALL_GETCWD: &str = "17";
const LINUX_SYSCALL_MKDIRAT: &str = "34";
const LINUX_SYSCALL_UNLINKAT: &str = "35";
const LINUX_SYSCALL_CHDIR: &str = "49";
const LINUX_SYSCALL_OPENAT: &str = "56";
const LINUX_SYSCALL_CLOSE: &str = "57";
const LINUX_SYSCALL_LSEEK: &str = "62";
const LINUX_SYSCALL_FSYNC: &str = "82";
const LINUX_SYSCALL_GETDENTS64: &str = "61";
const LINUX_SYSCALL_READ: &str = "63";
const LINUX_SYSCALL_PPOLL: &str = "73";
const LINUX_SYSCALL_READLINKAT: &str = "78";
const LINUX_SYSCALL_RENAMEAT: &str = "38";
const LINUX_SYSCALL_FACCESSAT: &str = "48";
const LINUX_SYSCALL_NEWFSTATAT: &str = "79";
const LINUX_SYSCALL_GETRANDOM: &str = "278";
const LINUX_AT_FDCWD: &str = "18446744073709551516";
const LINUX_O_CLOEXEC: usize = 524288;
const LINUX_O_CREAT: usize = 64;
const LINUX_O_DIRECTORY: usize = 16384;
const LINUX_O_EXCL: usize = 128;
const LINUX_O_PATH: usize = 2097152;
const LINUX_O_RDWR: usize = 2;
const LINUX_EEXIST_NEGATIVE: &str = "18446744073709551599";
const LINUX_DIRENT_STATE_SIZE: usize = 24 + LINUX_DIRENT_BUFFER_SIZE + LINUX_DIRENT_SYNTHETIC_SIZE;
const LINUX_DIRENT_BUFFER_SIZE: usize = 2048;
const LINUX_DIRENT_SYNTHETIC_SIZE: usize = 512;
const LINUX_DIRENT_STATE_FD_OFFSET: usize = 0;
const LINUX_DIRENT_STATE_CURSOR_OFFSET: usize = 8;
const LINUX_DIRENT_STATE_LEN_OFFSET: usize = 16;
const LINUX_DIRENT_STATE_BUFFER_OFFSET: usize = 24;
const LINUX_DIRENT_SYNTHETIC_OFFSET: usize =
    LINUX_DIRENT_STATE_BUFFER_OFFSET + LINUX_DIRENT_BUFFER_SIZE;
const SYNTHETIC_DIRENT_NAMLEN_OFFSET: usize = 18;
const SYNTHETIC_DIRENT_NAME_OFFSET: usize = 21;

impl code::CodegenPlatform for Platform {
    fn target(&self) -> &'static str {
        "linux-aarch64"
    }

    fn arch(&self) -> &'static str {
        "aarch64"
    }

    fn preserves_link_register_in_runtime_helpers(&self) -> bool {
        false
    }

    fn emit_program_exit(
        &self,
        _from: &str,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", "93"),
            abi::syscall(),
            abi::branch_self(),
            abi::return_(),
        ]);
        Ok(())
    }

    fn emit_write(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", "64"),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_poll_input(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate("x3", "Integer", "0"),
            abi::move_immediate("x4", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_PPOLL),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_path_exists(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register("x1", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_immediate("x2", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_FACCESSAT),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_path_stat(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register("x2", "x1"),
            abi::move_register("x1", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_immediate("x3", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_NEWFSTATAT),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn stat_mode_offset(&self) -> usize {
        16
    }

    fn emit_current_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_GETCWD),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_fs_path_operation(
        &self,
        _from: &str,
        operation: code::FsPathOperation,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        match operation {
            code::FsPathOperation::Chdir => {
                instructions.extend([
                    abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CHDIR),
                    abi::syscall(),
                ]);
            }
            code::FsPathOperation::Unlink => {
                instructions.extend([
                    abi::move_register("x1", abi::return_register()),
                    abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
                    abi::move_immediate("x2", "Integer", "0"),
                    abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_UNLINKAT),
                    abi::syscall(),
                ]);
            }
            code::FsPathOperation::Mkdir => {
                instructions.extend([
                    abi::move_register("x1", abi::return_register()),
                    abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
                    abi::move_immediate("x2", "Integer", "493"),
                    abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MKDIRAT),
                    abi::syscall(),
                ]);
            }
            code::FsPathOperation::Rmdir => {
                instructions.extend([
                    abi::move_register("x1", abi::return_register()),
                    abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
                    abi::move_immediate("x2", "Integer", "512"),
                    abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_UNLINKAT),
                    abi::syscall(),
                ]);
            }
        }
        Ok(())
    }

    fn emit_errno(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.push(abi::subtract_registers("x9", "x31", abi::return_register()));
        Ok(())
    }

    fn emit_open_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register("x3", "x2"),
            abi::move_register("x2", "x1"),
            abi::move_register("x1", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_OPENAT),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_read_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_READ),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_close_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_sync_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_FSYNC),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_seek_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_LSEEK),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_rename_path(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register("x3", "x1"),
            abi::move_register("x1", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_immediate("x2", "Integer", LINUX_AT_FDCWD),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_RENAMEAT),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_mkstemps(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let suffix = format!("{from}_mkstemps_suffix");
        let suffix_loop = format!("{from}_mkstemps_suffix_loop");
        let attempt = format!("{from}_mkstemps_attempt");
        let fill_done = format!("{from}_mkstemps_fill_done");
        let open_ok = format!("{from}_mkstemps_open_ok");
        let retry = format!("{from}_mkstemps_retry");
        let fail = format!("{from}_mkstemps_fail");

        instructions.extend([
            abi::move_register("x17", abi::return_register()),
            abi::move_register("x16", "x1"),
            abi::move_register("x9", "x17"),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&suffix_loop),
            abi::load_u8("x11", "x9", 0),
            abi::compare_immediate("x11", "0"),
            abi::branch_eq(&suffix),
            abi::add_immediate("x9", "x9", 1),
            abi::add_immediate("x10", "x10", 1),
            abi::branch(&suffix_loop),
            abi::label(&suffix),
            abi::subtract_registers("x9", "x10", "x16"),
            abi::subtract_immediate("x9", "x9", 6),
            abi::add_registers("x11", "x17", "x9"),
            abi::move_immediate("x15", "Integer", "0"),
            abi::label(&attempt),
            abi::move_immediate("x9", "Integer", "1000000"),
            abi::compare_registers("x15", "x9"),
            abi::branch_ge(&fail),
            abi::move_register("x12", "x15"),
            abi::move_immediate("x13", "Integer", "26"),
            abi::move_immediate("x14", "Integer", "0"),
        ]);
        for offset in 0..6 {
            let next = if offset == 5 {
                fill_done.clone()
            } else {
                format!("{from}_mkstemps_fill_{offset}")
            };
            instructions.extend([
                abi::unsigned_divide_registers("x9", "x12", "x13"),
                abi::multiply_subtract_registers("x10", "x9", "x13", "x12"),
                abi::add_immediate("x10", "x10", 97),
                abi::store_u8("x10", "x11", offset),
                abi::move_register("x12", "x9"),
                abi::label(&next),
            ]);
        }
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_register("x1", "x17"),
            abi::move_immediate(
                "x2",
                "Integer",
                &(LINUX_O_RDWR | LINUX_O_CREAT | LINUX_O_EXCL | LINUX_O_CLOEXEC).to_string(),
            ),
            abi::move_immediate("x3", "Integer", "384"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_OPENAT),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
            abi::move_immediate("x9", "Integer", LINUX_EEXIST_NEGATIVE),
            abi::compare_registers(abi::return_register(), "x9"),
            abi::branch_eq(&retry),
            abi::branch(&open_ok),
            abi::label(&retry),
            abi::add_immediate("x15", "x15", 1),
            abi::branch(&attempt),
            abi::label(&fail),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_EEXIST_NEGATIVE),
            abi::label(&open_ok),
        ]);
        Ok(())
    }

    fn emit_random_bytes(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate("x2", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_GETRANDOM),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_temp_directory(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let open_ok = format!("{from}_tmpdir_env_open_ok");
        let read_ok = format!("{from}_tmpdir_env_read_ok");
        let scan_entry = format!("{from}_tmpdir_scan_entry");
        let next_entry = format!("{from}_tmpdir_next_entry");
        let skip_entry = format!("{from}_tmpdir_skip_entry");
        let value_len_loop = format!("{from}_tmpdir_value_len_loop");
        let value_len_done = format!("{from}_tmpdir_value_len_done");
        let copy_loop = format!("{from}_tmpdir_copy_loop");
        let copy_done = format!("{from}_tmpdir_copy_done");
        let fallback = format!("{from}_tmpdir_fallback");
        let done = format!("{from}_tmpdir_done");

        let proc_path = b"/proc/self/environ\0";
        instructions.extend([
            abi::move_register("x17", abi::return_register()),
            abi::move_register("x16", "x1"),
        ]);
        for (offset, byte) in proc_path.iter().enumerate() {
            instructions.extend([
                abi::move_immediate("x9", "Byte", &byte.to_string()),
                abi::store_u8("x9", "x17", offset),
            ]);
        }
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_register("x1", "x17"),
            abi::move_immediate("x2", "Integer", &LINUX_O_CLOEXEC.to_string()),
            abi::move_immediate("x3", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_OPENAT),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
            abi::branch(&fallback),
            abi::label(&open_ok),
            abi::move_register("x15", abi::return_register()),
            abi::move_register("x1", "x17"),
            abi::move_register("x2", "x16"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_READ),
            abi::syscall(),
            abi::move_register("x14", abi::return_register()),
            abi::move_register(abi::return_register(), "x15"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
            abi::compare_immediate("x14", "0"),
            abi::branch_gt(&read_ok),
            abi::branch(&fallback),
            abi::label(&read_ok),
            abi::move_register("x10", "x17"),
            abi::add_registers("x11", "x17", "x14"),
            abi::label(&scan_entry),
            abi::compare_registers("x10", "x11"),
            abi::branch_ge(&fallback),
            abi::load_u8("x9", "x10", 0),
            abi::compare_immediate("x9", "0"),
            abi::branch_eq(&next_entry),
            abi::load_u8("x9", "x10", 0),
            abi::compare_immediate("x9", "84"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 1),
            abi::compare_immediate("x9", "77"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 2),
            abi::compare_immediate("x9", "80"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 3),
            abi::compare_immediate("x9", "68"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 4),
            abi::compare_immediate("x9", "73"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 5),
            abi::compare_immediate("x9", "82"),
            abi::branch_ne(&skip_entry),
            abi::load_u8("x9", "x10", 6),
            abi::compare_immediate("x9", "61"),
            abi::branch_ne(&skip_entry),
            abi::add_immediate("x12", "x10", 7),
            abi::move_register("x13", "x12"),
            abi::branch(&value_len_loop),
            abi::label(&skip_entry),
            abi::load_u8("x9", "x10", 0),
            abi::compare_immediate("x9", "0"),
            abi::branch_eq(&next_entry),
            abi::add_immediate("x10", "x10", 1),
            abi::compare_registers("x10", "x11"),
            abi::branch_lt(&skip_entry),
            abi::branch(&fallback),
            abi::label(&next_entry),
            abi::add_immediate("x10", "x10", 1),
            abi::branch(&scan_entry),
            abi::label(&value_len_loop),
            abi::compare_registers("x13", "x11"),
            abi::branch_ge(&value_len_done),
            abi::load_u8("x9", "x13", 0),
            abi::compare_immediate("x9", "0"),
            abi::branch_eq(&value_len_done),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&value_len_loop),
            abi::label(&value_len_done),
            abi::subtract_registers("x14", "x13", "x12"),
            abi::compare_immediate("x14", "0"),
            abi::branch_eq(&fallback),
            abi::move_immediate("x15", "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers("x15", "x14"),
            abi::branch_eq(&copy_done),
            abi::load_u8("x9", "x12", 0),
            abi::store_u8("x9", "x17", 0),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x17", "x17", 1),
            abi::add_immediate("x15", "x15", 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8("x31", "x17", 0),
            abi::move_register(abi::return_register(), "x14"),
            abi::branch(&done),
            abi::label(&fallback),
        ]);
        for (offset, byte) in b"/tmp\0".iter().enumerate() {
            instructions.extend([
                abi::move_immediate("x9", "Byte", &byte.to_string()),
                abi::store_u8("x9", "x17", offset),
            ]);
        }
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "4"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_opendir(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let suffix = instructions.len();
        let map_ok = format!("{from}_opendir_{suffix}_map_ok");
        let open_ok = format!("{from}_opendir_{suffix}_open_ok");
        let map_fail = format!("{from}_opendir_{suffix}_map_fail");
        let done = format!("{from}_opendir_{suffix}_done");
        instructions.extend([
            abi::move_register("x1", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_immediate(
                "x2",
                "Integer",
                &(LINUX_O_DIRECTORY | LINUX_O_CLOEXEC).to_string(),
            ),
            abi::move_immediate("x3", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_OPENAT),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
            abi::branch(&done),
            abi::label(&open_ok),
            abi::move_register("x17", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_immediate("x1", "Integer", &LINUX_DIRENT_STATE_SIZE.to_string()),
            abi::move_immediate("x2", "Integer", LINUX_PROT_READ_WRITE),
            abi::move_immediate("x3", "Integer", LINUX_MAP_PRIVATE_ANON),
            abi::move_immediate("x4", "Integer", &u64::MAX.to_string()),
            abi::move_immediate("x5", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MMAP),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&map_ok),
            abi::move_register(abi::return_register(), "x17"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
            abi::label(&map_fail),
            abi::move_immediate(abi::return_register(), "Integer", "18446744073709551604"),
            abi::branch(&done),
            abi::label(&map_ok),
            abi::move_register("x16", abi::return_register()),
            abi::store_u64("x17", "x16", LINUX_DIRENT_STATE_FD_OFFSET),
            abi::store_u64("x31", "x16", LINUX_DIRENT_STATE_CURSOR_OFFSET),
            abi::store_u64("x31", "x16", LINUX_DIRENT_STATE_LEN_OFFSET),
            abi::move_register(abi::return_register(), "x16"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_readdir(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let suffix = instructions.len();
        let have_entry = format!("{from}_readdir_{suffix}_have_entry");
        let refill_ok = format!("{from}_readdir_{suffix}_refill_ok");
        let scan_loop = format!("{from}_readdir_{suffix}_scan_loop");
        let scan_done = format!("{from}_readdir_{suffix}_scan_done");
        let copy_loop = format!("{from}_readdir_{suffix}_copy_loop");
        let copy_done = format!("{from}_readdir_{suffix}_copy_done");
        let done = format!("{from}_readdir_{suffix}_done");
        instructions.extend([
            abi::move_register("x17", abi::return_register()),
            abi::load_u64("x10", "x17", LINUX_DIRENT_STATE_CURSOR_OFFSET),
            abi::load_u64("x11", "x17", LINUX_DIRENT_STATE_LEN_OFFSET),
            abi::compare_registers("x10", "x11"),
            abi::branch_lt(&have_entry),
            abi::load_u64(abi::return_register(), "x17", LINUX_DIRENT_STATE_FD_OFFSET),
            abi::add_immediate("x1", "x17", LINUX_DIRENT_STATE_BUFFER_OFFSET),
            abi::move_immediate("x2", "Integer", &LINUX_DIRENT_BUFFER_SIZE.to_string()),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_GETDENTS64),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_gt(&refill_ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&refill_ok),
            abi::store_u64(abi::return_register(), "x17", LINUX_DIRENT_STATE_LEN_OFFSET),
            abi::store_u64("x31", "x17", LINUX_DIRENT_STATE_CURSOR_OFFSET),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&have_entry),
            abi::add_immediate("x12", "x17", LINUX_DIRENT_STATE_BUFFER_OFFSET),
            abi::add_registers("x12", "x12", "x10"),
            abi::load_u16("x13", "x12", 16),
            abi::add_registers("x14", "x10", "x13"),
            abi::store_u64("x14", "x17", LINUX_DIRENT_STATE_CURSOR_OFFSET),
            abi::add_immediate("x12", "x12", 19),
            abi::move_immediate("x14", "Integer", &LINUX_DIRENT_SYNTHETIC_OFFSET.to_string()),
            abi::add_registers("x14", "x17", "x14"),
            abi::add_immediate("x15", "x14", SYNTHETIC_DIRENT_NAME_OFFSET),
            abi::move_immediate("x13", "Integer", "0"),
            abi::label(&scan_loop),
            abi::load_u8("x16", "x12", 0),
            abi::compare_immediate("x16", "0"),
            abi::branch_eq(&scan_done),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&scan_loop),
            abi::label(&scan_done),
            abi::store_u8("x13", "x14", SYNTHETIC_DIRENT_NAMLEN_OFFSET),
            abi::store_u8("x31", "x14", SYNTHETIC_DIRENT_NAMLEN_OFFSET + 1),
            abi::subtract_registers("x12", "x12", "x13"),
            abi::move_immediate("x16", "Integer", "0"),
            abi::label(&copy_loop),
            abi::compare_registers("x16", "x13"),
            abi::branch_eq(&copy_done),
            abi::load_u8("x10", "x12", 0),
            abi::store_u8("x10", "x15", 0),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x15", "x15", 1),
            abi::add_immediate("x16", "x16", 1),
            abi::branch(&copy_loop),
            abi::label(&copy_done),
            abi::store_u8("x31", "x15", 0),
            abi::move_register(abi::return_register(), "x14"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_closedir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        instructions.extend([
            abi::move_register("x16", abi::return_register()),
            abi::load_u64(abi::return_register(), "x16", LINUX_DIRENT_STATE_FD_OFFSET),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
            abi::move_register(abi::return_register(), "x16"),
            abi::move_immediate("x1", "Integer", &LINUX_DIRENT_STATE_SIZE.to_string()),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn dirent_name_offset(&self) -> usize {
        SYNTHETIC_DIRENT_NAME_OFFSET
    }

    fn dirent_name_length_offset(&self) -> usize {
        SYNTHETIC_DIRENT_NAMLEN_OFFSET
    }

    fn emit_realpath(
        &self,
        from: &str,
        _platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String> {
        let suffix = instructions.len();
        let open_ok = format!("{from}_realpath_{suffix}_open_ok");
        let alloc_ok = format!("{from}_realpath_{suffix}_alloc_ok");
        let digit_loop = format!("{from}_realpath_{suffix}_digit_loop");
        let digit_done = format!("{from}_realpath_{suffix}_digit_done");
        let copy_digits = format!("{from}_realpath_{suffix}_copy_digits");
        let copy_done = format!("{from}_realpath_{suffix}_copy_done");
        let read_ok = format!("{from}_realpath_{suffix}_read_ok");
        let close_and_fail = format!("{from}_realpath_{suffix}_close_and_fail");
        let done = format!("{from}_realpath_{suffix}_done");
        instructions.extend([
            abi::move_register("x16", "x1"),
            abi::move_register("x17", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_register("x1", "x17"),
            abi::move_immediate(
                "x2",
                "Integer",
                &(LINUX_O_PATH | LINUX_O_CLOEXEC).to_string(),
            ),
            abi::move_immediate("x3", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_OPENAT),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&open_ok),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::branch(&done),
            abi::label(&open_ok),
            abi::move_register("x17", abi::return_register()),
            abi::move_immediate(abi::return_register(), "Integer", "64"),
            abi::move_immediate("x1", "Integer", "1"),
            abi::branch_link(ARENA_ALLOC_SYMBOL),
        ]);
        relocations.push(CodeRelocation {
            from: from.to_string(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&alloc_ok),
            abi::branch(&close_and_fail),
            abi::label(&alloc_ok),
            abi::move_register("x15", "x1"),
        ]);
        for (offset, byte) in b"/proc/self/fd/".iter().copied().enumerate() {
            instructions.extend([
                abi::move_immediate("x9", "Byte", &byte.to_string()),
                abi::store_u8("x9", "x15", offset),
            ]);
        }
        instructions.extend([
            abi::add_immediate("x14", "x15", 63),
            abi::store_u8("x31", "x14", 0),
            abi::move_register("x12", "x17"),
            abi::move_immediate("x13", "Integer", "10"),
            abi::label(&digit_loop),
            abi::unsigned_divide_registers("x9", "x12", "x13"),
            abi::multiply_subtract_registers("x10", "x9", "x13", "x12"),
            abi::add_immediate("x10", "x10", 48),
            abi::subtract_immediate("x14", "x14", 1),
            abi::store_u8("x10", "x14", 0),
            abi::move_register("x12", "x9"),
            abi::compare_immediate("x12", "0"),
            abi::branch_ne(&digit_loop),
            abi::label(&digit_done),
            abi::add_immediate("x11", "x15", 14),
            abi::label(&copy_digits),
            abi::load_u8("x10", "x14", 0),
            abi::store_u8("x10", "x11", 0),
            abi::compare_immediate("x10", "0"),
            abi::branch_eq(&copy_done),
            abi::add_immediate("x14", "x14", 1),
            abi::add_immediate("x11", "x11", 1),
            abi::branch(&copy_digits),
            abi::label(&copy_done),
            abi::move_immediate(abi::return_register(), "Integer", LINUX_AT_FDCWD),
            abi::move_register("x1", "x15"),
            abi::move_register("x2", "x16"),
            abi::move_immediate("x3", "Integer", "4095"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_READLINKAT),
            abi::syscall(),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_ge(&read_ok),
            abi::branch(&close_and_fail),
            abi::label(&read_ok),
            abi::add_registers("x9", "x16", abi::return_register()),
            abi::store_u8("x31", "x9", 0),
            abi::move_register(abi::return_register(), "x17"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
            abi::move_register(abi::return_register(), "x16"),
            abi::branch(&done),
            abi::label(&close_and_fail),
            abi::move_register(abi::return_register(), "x17"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_CLOSE),
            abi::syscall(),
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::label(&done),
        ]);
        Ok(())
    }

    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::return_register(), "Integer", "0"),
            abi::move_register("x1", "x23"),
            abi::move_immediate("x2", "Integer", LINUX_PROT_READ_WRITE),
            abi::move_immediate("x3", "Integer", LINUX_MAP_PRIVATE_ANON),
            abi::move_immediate("x4", "Integer", &u64::MAX.to_string()),
            abi::move_immediate("x5", "Integer", "0"),
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MMAP),
            abi::syscall(),
        ]);
        Ok(())
    }

    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String> {
        instructions.extend([
            abi::move_immediate(abi::syscall_register(), "Integer", LINUX_SYSCALL_MUNMAP),
            abi::syscall(),
        ]);
        Ok(())
    }
}
