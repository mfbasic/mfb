use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::binary_repr::{self};
use crate::builtins;
use crate::json_string;
use crate::numeric;

use super::nir::{
    self, NirFunction, NirMatchPattern, NirModule, NirOp, NirRecordUpdate, NirSourceLoc, NirValue,
};
use super::plan::NativePlan;
use super::runtime;

const RESULT_OK_TAG: &str = "0";
const RESULT_ERR_TAG: &str = "1";
const RESULT_PROGRAM_EXIT_TAG: &str = "2";
const ERR_OVERFLOW_CODE: &str = "77050010";
const ERR_OVERFLOW_MESSAGE: &str = "numeric overflow";
const ERR_OVERFLOW_SYMBOL: &str = "_mfb_str_error_overflow";
const ERR_UNDERFLOW_CODE: &str = "77050011";
const ERR_UNDERFLOW_MESSAGE: &str = "numeric underflow";
const ERR_UNDERFLOW_SYMBOL: &str = "_mfb_str_error_underflow";
const ERR_FLOAT_DOMAIN_CODE: &str = "77050012";
const ERR_FLOAT_DOMAIN_MESSAGE: &str = "float domain error";
const ERR_FLOAT_DOMAIN_SYMBOL: &str = "_mfb_str_error_float_domain";
const ERR_FLOAT_NAN_CODE: &str = "77050013";
const ERR_FLOAT_NAN_MESSAGE: &str = "float NaN result";
const ERR_FLOAT_NAN_SYMBOL: &str = "_mfb_str_error_float_nan";
const ERR_FLOAT_INF_CODE: &str = "77050014";
const ERR_FLOAT_INF_MESSAGE: &str = "float infinity result";
const ERR_FLOAT_INF_SYMBOL: &str = "_mfb_str_error_float_inf";
const ERR_FLOAT_OVERFLOW_CODE: &str = "77050015";
const ERR_FLOAT_OVERFLOW_MESSAGE: &str = "float overflow";
const ERR_FLOAT_OVERFLOW_SYMBOL: &str = "_mfb_str_error_float_overflow";
const ERR_ALLOCATION_MESSAGE: &str = "allocation failed";
const ERR_ALLOCATION_SYMBOL: &str = "_mfb_str_error_allocation";
const ERR_INDEX_OUT_OF_RANGE_CODE: &str = "77050001";
const ERR_INDEX_OUT_OF_RANGE_MESSAGE: &str = "index out of range";
const ERR_INDEX_OUT_OF_RANGE_SYMBOL: &str = "_mfb_str_error_index_out_of_range";
const ERR_NOT_FOUND_CODE: &str = "77050004";
const ERR_NOT_FOUND_MESSAGE: &str = "not found";
const ERR_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_not_found";
const ERR_TIMEOUT_CODE: &str = "77050008";
const ERR_TIMEOUT_MESSAGE: &str = "timeout";
const ERR_TIMEOUT_SYMBOL: &str = "_mfb_str_error_timeout";
const ERR_INTERRUPTED_CODE: &str = "77050009";
const ERR_INTERRUPTED_MESSAGE: &str = "interrupted";
const ERR_INTERRUPTED_SYMBOL: &str = "_mfb_str_error_interrupted";
const ERR_READ_CODE: &str = "77020001";
const ERR_READ_MESSAGE: &str = "read failure";
const ERR_READ_SYMBOL: &str = "_mfb_str_error_read";
const ERR_OUTPUT_CODE: &str = "77020002";
const ERR_OUTPUT_MESSAGE: &str = "output failure";
const ERR_OUTPUT_SYMBOL: &str = "_mfb_str_error_output";
/// macOS app mode (plan-04-macos-app.md §6.6): the standard program-entry logic
/// (arena setup + language `main` + exit) is emitted under this symbol and runs
/// on the worker thread, while `_main` is the AppKit bootstrap.
pub(crate) const MACAPP_PROGRAM_SYMBOL: &str = "_mfb_macapp_program";
const ERR_UNSUPPORTED_CODE: &str = "77050007";
const ERR_UNSUPPORTED_MESSAGE: &str = "unsupported operation";
const ERR_UNSUPPORTED_SYMBOL: &str = "_mfb_str_error_unsupported";
const ERR_EOF_CODE: &str = "77020003";
const ERR_EOF_MESSAGE: &str = "end of file";
const ERR_EOF_SYMBOL: &str = "_mfb_str_error_eof";
const ERR_RESOURCE_CLOSED_CODE: &str = "77030004";
const ERR_RESOURCE_CLOSED_MESSAGE: &str = "resource closed";
const ERR_RESOURCE_CLOSED_SYMBOL: &str = "_mfb_str_error_resource_closed";
const ERR_NATIVE_LINK_LOAD_CODE: &str = "77030007";
const ERR_NATIVE_LINK_LOAD_MESSAGE: &str = "native binding library unavailable";
const ERR_NATIVE_LINK_LOAD_SYMBOL: &str = "_mfb_str_error_native_link_load";
const ERR_NATIVE_LINK_CALL_CODE: &str = "77030008";
const ERR_NATIVE_LINK_CALL_MESSAGE: &str = "native binding call failed";
const ERR_NATIVE_LINK_CALL_SYMBOL: &str = "_mfb_str_error_native_link_call";
const ERR_ENCODING_CODE: &str = "77020004";
const ERR_ENCODING_MESSAGE: &str = "invalid encoding";
const ERR_ENCODING_SYMBOL: &str = "_mfb_str_error_encoding";
const ERR_INPUT_CODE: &str = "77020005";
const ERR_INPUT_MESSAGE: &str = "input failure";
const ERR_INPUT_SYMBOL: &str = "_mfb_str_error_input";
const ENTRY_ERROR_PREFIX: &str = "Code: ";
const ENTRY_ERROR_PREFIX_SYMBOL: &str = "_mfb_str_entry_error_prefix";
const ENTRY_ERROR_SEPARATOR: &str = " Message: ";
const ENTRY_ERROR_SEPARATOR_SYMBOL: &str = "_mfb_str_entry_error_separator";
const ENTRY_ERROR_NEWLINE: &str = "\n";
const ENTRY_ERROR_NEWLINE_SYMBOL: &str = "_mfb_str_entry_error_newline";
const CLEANUP_FAILURE_PREFIX: &str = "Cleanup failure: Code: ";
const CLEANUP_FAILURE_PREFIX_SYMBOL: &str = "_mfb_str_cleanup_failure_prefix";
const CLEANUP_FAILURE_SEPARATOR: &str = " Message: ";
const CLEANUP_FAILURE_SEPARATOR_SYMBOL: &str = ENTRY_ERROR_SEPARATOR_SYMBOL;
const RESULT_TAG_REGISTER: &str = abi::RETURN_REGISTER;
const RESULT_VALUE_REGISTER: &str = "x1";
const RESULT_ERROR_MESSAGE_REGISTER: &str = "x2";
/// Fourth error-result register: pointer to the `ErrorLoc` recording where the
/// error originated. Carried alongside code (x1) and message (x2) so propagation
/// preserves the origin and trap materialization can build a 3-field `Error`.
const RESULT_ERROR_SOURCE_REGISTER: &str = "x3";
/// Byte size of an allocated `Error` record: code(+0), message(+8), source(+16).
const ERROR_OBJECT_SIZE: usize = 24;
/// Byte size of an allocated `ErrorLoc` record: filename(+0), line(+8), char(+16).
const ERROR_LOC_OBJECT_SIZE: usize = 24;
pub(crate) const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
const ARENA_DESTROY_SYMBOL: &str = "_mfb_arena_destroy";
/// Shared process-teardown routine: restores the terminal (when `term::` is used)
/// and frees the main arena, then returns. Called both after the entry FUNC/SUB
/// finishes and from the SIGINT/SIGTERM handler, so the cleanup is identical on a
/// normal exit and a signal kill. It locates the arena through
/// `MAIN_ARENA_GLOBAL_SYMBOL` (not `x19`) so it works from a signal handler whose
/// `x19` belongs to the interrupted code.
const SHUTDOWN_SYMBOL: &str = "_mfb_shutdown";
/// `void handler(int signo)` installed for SIGINT/SIGTERM in console programs. It
/// runs `_mfb_shutdown` and then `_exit(128 + signo)`; it never returns.
const SIGNAL_HANDLER_SYMBOL: &str = "_mfb_rt_signal_handler";
/// One writable 8-byte global holding the main thread's arena-state address,
/// stored at program startup. The signal handler and `_mfb_shutdown` read it to
/// find the arena without relying on the pinned `x19` (which is unavailable on a
/// signal frame). Per-thread worker arenas are intentionally not tracked here —
/// they are never freed by us anyway (the entry only ever frees the main arena).
const MAIN_ARENA_GLOBAL_SYMBOL: &str = "_mfb_rt_main_arena";
const ARENA_STATE_REGISTER: &str = "x19";
const CLOSURE_ENV_REGISTER: &str = "x28";
const CLOSURE_OBJECT_SIZE: usize = 16;
const CLOSURE_OFFSET_CODE: usize = 0;
const CLOSURE_OFFSET_ENV: usize = 8;
const ENTRY_STACK_SIZE: usize = 112;
const ENTRY_GLOBALS_OFFSET: usize = ENTRY_STACK_SIZE;
/// `term::` TUI-mode state slots reserved in the program-entry frame just past
/// the program globals and `LINK` slots (plan-01-term.md §6.2). Eight `u64`
/// slots: active, packed foreground, packed background, bold, underline,
/// cursor-visible, and two reserved for the app backend. Zero-initialized by the
/// entry's global-slot clear, which is the inert (TUI-off) default.
const TERM_STATE_SLOTS: usize = 8;
pub(crate) const TERM_STATE_ACTIVE_OFFSET: usize = 0;
pub(crate) const TERM_STATE_FG_OFFSET: usize = 8;
pub(crate) const TERM_STATE_BG_OFFSET: usize = 16;
pub(crate) const TERM_STATE_BOLD_OFFSET: usize = 24;
pub(crate) const TERM_STATE_UNDERLINE_OFFSET: usize = 32;
pub(crate) const TERM_STATE_CURSOR_VISIBLE_OFFSET: usize = 40;
const ARENA_CLEANUP_FAILURE_COUNT_OFFSET: usize = 64;
const ARENA_CLEANUP_FAILURE_CODE_OFFSET: usize = 72;
const ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET: usize = 80;
/// Per-arena (per-thread) PCG64 random-number generator state. Each OS thread
/// owns its own arena, so storing the 128-bit RNG state in the arena gives every
/// thread an independent stream reachable through the pinned arena register
/// (`x19`) without a thread-local lookup. Appended past the cleanup-audit fields
/// so the historical 0..88 layout is unchanged for programs that never seed.
const ARENA_RNG_STATE_LO_OFFSET: usize = 88;
const ARENA_RNG_STATE_HI_OFFSET: usize = 96;
const ARENA_STATE_SIZE: usize = 104;
/// Advance one PCG64 step and return the next 64-bit value in `x0`; reads/writes
/// the calling thread's arena RNG state via `x19`.
const RNG_NEXT_SYMBOL: &str = "_mfb_rng_next";
/// Seed the PCG64 state at `[x0 + ARENA_RNG_STATE_*]` from the 64-bit seed in
/// `x1`. Used both for the program-startup seed and to give each spawned thread
/// its own stream drawn from the parent's generator.
const RNG_SEED_SYMBOL: &str = "_mfb_rng_seed_at";
/// PCG64 (XSL-RR 128/64) default LCG multiplier, high and low 64-bit limbs.
const PCG_MULT_HI: u64 = 0x2360_ED05_1FC6_5DA4;
const PCG_MULT_LO: u64 = 0x4385_DF64_9FCC_F645;
/// PCG64 default stream increment, high and low 64-bit limbs.
const PCG_INC_HI: u64 = 0x5851_F42D_4C95_7F2D;
const PCG_INC_LO: u64 = 0x1405_7B7E_F767_814F;
const ENTRY_ARGC_OFFSET: usize = ARENA_STATE_SIZE;
const ENTRY_ARGV_OFFSET: usize = ENTRY_ARGC_OFFSET + 8;
const ENTRY_ARGS_LIST_OFFSET: usize = ENTRY_ARGV_OFFSET + 8;
const ENTRY_ARGS_DATA_LENGTH_OFFSET: usize = ENTRY_ARGS_LIST_OFFSET + 8;
const ENTRY_ARGS_COUNT_SAVED_OFFSET: usize = ENTRY_ARGS_DATA_LENGTH_OFFSET + 8;
const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
const ARENA_BLOCK_HEADER_SIZE: usize = 32;
const ERR_INVALID_ARGUMENT_CODE: &str = "77050002";
const ERR_INVALID_ARGUMENT_MESSAGE: &str = "invalid argument";
const ERR_INVALID_ARGUMENT_SYMBOL: &str = "_mfb_str_error_invalid_argument";
const ERR_INVALID_FORMAT_CODE: &str = "77050003";
const ERR_INVALID_FORMAT_MESSAGE: &str = "invalid format";
const ERR_INVALID_FORMAT_SYMBOL: &str = "_mfb_str_error_invalid_format";
const ERR_OUT_OF_MEMORY_CODE: &str = "77010001";
const ERR_ALREADY_EXISTS_CODE: &str = "77050005";
const ERR_ALREADY_EXISTS_MESSAGE: &str = "already exists";
const ERR_ALREADY_EXISTS_SYMBOL: &str = "_mfb_str_error_already_exists";
const ERR_ACCESS_DENIED_CODE: &str = "77030003";
const ERR_ACCESS_DENIED_MESSAGE: &str = "access denied";
const ERR_ACCESS_DENIED_SYMBOL: &str = "_mfb_str_error_access_denied";
const ERR_DIRECTORY_NOT_EMPTY_CODE: &str = "77030005";
const ERR_DIRECTORY_NOT_EMPTY_MESSAGE: &str = "directory not empty";
const ERR_DIRECTORY_NOT_EMPTY_SYMBOL: &str = "_mfb_str_error_directory_not_empty";
const ERR_CLOSE_FAILED_CODE: &str = "77030006";
const ERR_CLOSE_FAILED_MESSAGE: &str = "close failed";
const ERR_CLOSE_FAILED_SYMBOL: &str = "_mfb_str_error_close_failed";
const ERR_PATH_NOT_FOUND_CODE: &str = "77030001";
const ERR_PATH_NOT_FOUND_MESSAGE: &str = "path not found";
const ERR_PATH_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_path_not_found";
const ERR_INVALID_PATH_CODE: &str = "77030002";
const ERR_INVALID_PATH_MESSAGE: &str = "invalid path";
const ERR_INVALID_PATH_SYMBOL: &str = "_mfb_str_error_invalid_path";
const ERR_ADDRESS_INVALID_CODE: &str = "77070001";
const ERR_ADDRESS_INVALID_MESSAGE: &str = "address invalid";
const ERR_ADDRESS_INVALID_SYMBOL: &str = "_mfb_str_error_address_invalid";
const ERR_ADDRESS_NOT_FOUND_CODE: &str = "77070002";
const ERR_ADDRESS_NOT_FOUND_MESSAGE: &str = "address not found";
const ERR_ADDRESS_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_address_not_found";
const ERR_NETWORK_FAILED_CODE: &str = "77070003";
const ERR_NETWORK_FAILED_MESSAGE: &str = "network failed";
const ERR_NETWORK_FAILED_SYMBOL: &str = "_mfb_str_error_network_failed";
const ERR_CONNECTION_CLOSED_CODE: &str = "77070004";
const ERR_CONNECTION_CLOSED_MESSAGE: &str = "connection closed";
const ERR_CONNECTION_CLOSED_SYMBOL: &str = "_mfb_str_error_connection_closed";
const ERR_READ_TIMEOUT_CODE: &str = "77070005";
const ERR_READ_TIMEOUT_MESSAGE: &str = "read timeout";
const ERR_READ_TIMEOUT_SYMBOL: &str = "_mfb_str_error_read_timeout";
const ERR_WRITE_TIMEOUT_CODE: &str = "77070006";
const ERR_WRITE_TIMEOUT_MESSAGE: &str = "write timeout";
const ERR_WRITE_TIMEOUT_SYMBOL: &str = "_mfb_str_error_write_timeout";
const ERR_MESSAGE_TOO_LARGE_CODE: &str = "77070007";
const ERR_MESSAGE_TOO_LARGE_MESSAGE: &str = "message too large";
const ERR_MESSAGE_TOO_LARGE_SYMBOL: &str = "_mfb_str_error_message_too_large";
const ERR_TLS_FAILED_CODE: &str = "77070008";
const ERR_TLS_FAILED_MESSAGE: &str = "TLS failed";
const ERR_TLS_FAILED_SYMBOL: &str = "_mfb_str_error_tls_failed";
const EMPTY_STRING_SYMBOL: &str = "_mfb_str_empty";
const FS_MODE_TYPE_MASK: &str = "61440";
const FS_MODE_DIRECTORY: &str = "16384";
const FS_MODE_REGULAR: &str = "32768";
const FILE_OFFSET_FD: usize = 0;
const FILE_OFFSET_CLOSED: usize = 8;
/// Offset of the optional `STATE` payload pointer in a resource record. A
/// resource value is a pointer to its arena record, so a borrow shares the same
/// record and therefore the same `STATE`. The slot is null until the owning
/// `RES` binding default-initializes it.
pub(crate) const FILE_OFFSET_STATE: usize = 16;
/// Size of a resource record: fd, closed flag, and the `STATE` pointer.
pub(crate) const RESOURCE_RECORD_SIZE: &str = "24";
const COLLECTION_KIND_LIST: usize = 0;
const COLLECTION_KIND_MAP: usize = 1;
const COLLECTION_HEADER_SIZE: usize = 40;
const COLLECTION_OFFSET_KIND: usize = 0;
const COLLECTION_OFFSET_KEY_TYPE: usize = 1;
const COLLECTION_OFFSET_VALUE_TYPE: usize = 2;
const COLLECTION_OFFSET_FLAGS_VERSION: usize = 3;
const COLLECTION_OFFSET_COUNT: usize = 8;
const COLLECTION_OFFSET_CAPACITY: usize = 16;
const COLLECTION_OFFSET_DATA_LENGTH: usize = 24;
const COLLECTION_OFFSET_DATA_CAPACITY: usize = 32;
const COLLECTION_ENTRY_SIZE: usize = 40;
const COLLECTION_ENTRY_OFFSET_FLAGS: usize = 0;
const COLLECTION_ENTRY_OFFSET_KEY_OFFSET: usize = 8;
const COLLECTION_ENTRY_OFFSET_KEY_LENGTH: usize = 16;
const COLLECTION_ENTRY_OFFSET_VALUE_OFFSET: usize = 24;
const COLLECTION_ENTRY_OFFSET_VALUE_LENGTH: usize = 32;
const COLLECTION_ENTRY_FLAG_USED: usize = 1;
const COLLECTION_TYPE_NONE: usize = 0;
const COLLECTION_TYPE_BOOLEAN: usize = 2;
const COLLECTION_TYPE_INTEGER: usize = 3;
const COLLECTION_TYPE_FLOAT: usize = 4;
const COLLECTION_TYPE_FIXED: usize = 5;
const COLLECTION_TYPE_STRING: usize = 6;
const COLLECTION_TYPE_BYTE: usize = 7;
const COLLECTION_TYPE_LIST: usize = 20;
const COLLECTION_TYPE_MAP: usize = 21;
const COLLECTION_TYPE_OBJECT: usize = 22;
const UNICODE_STAGE1_SYMBOL: &str = "_mfb_unicode_stage1";
const UNICODE_STAGE2_SYMBOL: &str = "_mfb_unicode_stage2";
const UNICODE_PROPERTIES_SYMBOL: &str = "_mfb_unicode_properties";
const UNICODE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_sequences";
const UNICODE_COMBINATIONS_SECOND_SYMBOL: &str = "_mfb_unicode_combinations_second";
const UNICODE_COMBINATIONS_COMBINED_SYMBOL: &str = "_mfb_unicode_combinations_combined";
const UNICODE_NFD_ENTRIES_SYMBOL: &str = "_mfb_unicode_nfd_entries";
const UNICODE_NFD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_nfd_sequences";
const UNICODE_UPPERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_uppercase_entries";
const UNICODE_UPPERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_uppercase_sequences";
const UNICODE_LOWERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_lowercase_entries";
const UNICODE_LOWERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_lowercase_sequences";
const UNICODE_CASEFOLD_ENTRIES_SYMBOL: &str = "_mfb_unicode_casefold_entries";
const UNICODE_CASEFOLD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_casefold_sequences";
const THREAD_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_thread_trampoline";
const FLOAT_TO_STRING_FORMAT: &str = "%.*f";
const FLOAT_TO_STRING_BUFFER_SIZE: usize = 640;

pub(crate) struct NativeCodePlan {
    pub(crate) target: String,
    /// Native build mode this code plan was lowered for (`console` or
    /// `macos-app`), carried from the NIR module / native plan.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) arch: String,
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) imports: Vec<CodeImport>,
    pub(crate) data_objects: Vec<CodeDataObject>,
    pub(crate) functions: Vec<CodeFunction>,
}

pub(crate) struct CodeFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) params: Vec<CodeParam>,
    pub(crate) returns: String,
    pub(crate) frame: CodeFrame,
    pub(crate) instructions: Vec<CodeInstruction>,
    pub(crate) relocations: Vec<CodeRelocation>,
    pub(crate) stack_slots: Vec<CodeStackSlot>,
}

pub(crate) struct CodeFrame {
    pub(crate) stack_size: usize,
    pub(crate) callee_saved: Vec<String>,
}

pub(crate) struct CodeParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) location: String,
}

pub(crate) struct CodeInstruction {
    pub(crate) op: CodeOp,
    pub(crate) fields: Vec<(&'static str, String)>,
}

pub(crate) struct CodeRelocation {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

pub(crate) struct CodeImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
}

pub(crate) struct CodeDataObject {
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) layout: String,
    pub(crate) align: usize,
    pub(crate) size: usize,
    pub(crate) value: String,
}

pub(crate) trait CodegenPlatform {
    fn target(&self) -> &'static str;
    fn arch(&self) -> &'static str;
    fn preserves_link_register_in_runtime_helpers(&self) -> bool;
    fn termios_size(&self) -> usize;
    fn termios_lflag_offset(&self) -> usize;
    fn termios_lflag_width(&self) -> usize;
    fn termios_cc_offset(&self) -> usize;
    fn termios_echo_flag(&self) -> u64;
    fn termios_icanon_flag(&self) -> u64;
    fn termios_vmin_index(&self) -> usize;
    fn termios_vtime_index(&self) -> usize;
    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_poll_input(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_is_terminal(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_terminal_size(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_path_exists(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_path_stat(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn stat_mode_offset(&self) -> usize;
    fn emit_current_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_fs_path_operation(
        &self,
        from: &str,
        operation: FsPathOperation,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_errno(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    /// Emit a `bl` to a libc function named by its platform-independent base
    /// name (e.g. `socket`, `getaddrinfo`). macOS prepends a leading `_`
    /// (libSystem); Linux uses the name verbatim (libc). Arguments must already
    /// be in `x0..`, the result is returned in `x0`. Used by the `net` runtime
    /// helpers, which marshal socket calls onto libc.
    fn emit_libc_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_open_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_read_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_close_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_sync_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_seek_file(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_rename_path(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_mkstemps(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_random_bytes(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_temp_directory(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_opendir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_readdir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_closedir(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn dirent_name_offset(&self) -> usize;
    fn dirent_name_length_offset(&self) -> usize;
    fn emit_realpath(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
    /// Byte offset of `ai_addr` within `struct addrinfo`. macOS orders
    /// `ai_canonname` before `ai_addr` (offset 32); Linux orders `ai_addr` first
    /// (offset 24).
    fn addrinfo_addr_offset(&self) -> usize;
    /// `setsockopt` level/option constants, which differ between platforms.
    fn sol_socket(&self) -> &'static str;
    fn so_reuseaddr(&self) -> &'static str;
    fn so_rcvtimeo(&self) -> &'static str;
    fn so_sndtimeo(&self) -> &'static str;
    /// `EAGAIN`/`EWOULDBLOCK` errno value, used to distinguish a socket
    /// read/write timeout from a connection failure.
    fn eagain(&self) -> &'static str;
    /// `EMSGSIZE` errno value, used to map an oversized datagram `sendto`
    /// failure to `ErrMessageTooLarge`.
    fn emsgsize(&self) -> &'static str;
    /// `O_NONBLOCK` open/`fcntl` flag, `EINPROGRESS` errno, and `SO_ERROR`
    /// socket option, used by the non-blocking `connect` + `poll` timeout path.
    fn o_nonblock(&self) -> &'static str;
    fn einprogress(&self) -> &'static str;
    fn so_error(&self) -> &'static str;
    /// Emit a `bl` to a libc function that takes a single trailing variadic
    /// argument in `x2` (e.g. `open(path, flags, mode)`, `fcntl(fd, cmd, arg)`).
    /// On the Darwin AArch64 ABI variadic arguments are passed on the stack, so
    /// the value in `x2` is spilled to the stack top across the call; on Linux it
    /// is passed in `x2` like a normal argument. Result is returned in `x0`.
    fn emit_variadic_call(
        &self,
        base: &str,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;

    /// Emit the macOS app-mode (`NativeBuildMode::MacApp`) `_main` AppKit
    /// bootstrap and any supporting functions (e.g. the pthread worker shim).
    /// The standard program-entry logic is emitted separately under
    /// [`MACAPP_PROGRAM_SYMBOL`] and runs on the spawned worker thread.
    ///
    /// Returns `None` for targets without app mode (the caller then reports that
    /// app mode is unsupported); `Some(Ok(functions))` for the macOS backend.
    fn emit_app_program_entry(
        &self,
        _spec: &AppEntrySpec,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<Vec<CodeFunction>, String>> {
        None
    }

    /// Read-only data objects (Obj-C class/selector C strings, window title,
    /// env-var names) referenced by the app-mode bootstrap. Empty otherwise.
    fn app_mode_data_objects(&self) -> Vec<CodeDataObject> {
        Vec::new()
    }

    /// App-mode body for `io.print`/`io.write`/`io.printError`/`io.writeError`
    /// (plan-04-macos-app.md §5.4): append the string to the AppKit transcript,
    /// falling back to the file descriptor when no window is attached (headless).
    /// `None` for targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_write_helper(
        &self,
        _symbol: &str,
        _stderr: bool,
        _newline: bool,
        _term_state_offset: Option<usize>,
        _platform_imports: &HashMap<String, String>,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for `io.flush`/`io.flushError`. `None` for non-app targets.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_flush_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for `io.input` (plan §5.4): write the prompt to the
    /// transcript, then read a line from the window input pipe. `None` for
    /// targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_input_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode setup for immediate, no-echo key reads. `None` for non-app
    /// targets.
    fn emit_app_raw_input_mode(
        &self,
        _symbol: &str,
        _instructions: &mut Vec<CodeInstruction>,
        _relocations: &mut Vec<CodeRelocation>,
    ) -> Option<Result<(), String>> {
        None
    }

    /// App-mode body for `io.isInputTerminal`/`io.isOutputTerminal`/
    /// `io.isErrorTerminal` (plan §5.4): the window is the interactive console,
    /// so all three return TRUE. `None` for targets without app mode.
    #[allow(clippy::type_complexity)]
    fn emit_app_io_is_terminal_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for the transcript viewport size in text columns/rows,
    /// computed from the scroll view's content size and the monospaced font
    /// metrics. `None` for targets without app mode. Retained for plan-01-term.md
    /// Phase 5 (`term::terminalSize` app backend, §8.3); unused since
    /// `io::terminalSize` was removed in Phase 3.
    #[allow(clippy::type_complexity, dead_code)]
    fn emit_app_io_terminal_size_helper(
        &self,
        _symbol: &str,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }

    /// App-mode body for a `term::` runtime helper that drives the synthesized
    /// TermView surface (plan-01-term.md §6.3, Phase 4-5). Returns `None` for
    /// calls that keep the shared console backend (and for targets without app
    /// mode).
    #[allow(clippy::type_complexity)]
    fn emit_app_term_helper(
        &self,
        _call: &str,
        _symbol: &str,
        _term_state_offset: usize,
    ) -> Option<Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String>> {
        None
    }
}

/// Inputs the app-mode `_main` bootstrap needs about the program it hosts
/// (plan-04-macos-app.md §6.6). The worker thread runs the standard program
/// entry generated separately under [`MACAPP_PROGRAM_SYMBOL`]; the bootstrap
/// itself only needs to know whether to forward `argc`/`argv` to that entry.
pub(crate) struct AppEntrySpec {
    pub(crate) language_entry_accepts_args: bool,
    /// Whether the program uses `term::` (so the app-mode finish path should
    /// auto-`term::off()` to restore the transcript, plan-01-term.md §6.5).
    pub(crate) uses_term: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum FsPathOperation {
    Chdir,
    Unlink,
    Mkdir,
    Rmdir,
}

pub(crate) struct CodeStackSlot {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) offset: i32,
}

struct CodeBuilder<'a> {
    current_symbol: String,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, String>,
    platform_imports: &'a HashMap<String, String>,
    globals: &'a HashMap<String, GlobalValue>,
    type_model: TypeModel,
    string_symbols: &'a HashMap<String, String>,
    locals: HashMap<String, LocalValue>,
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    stack_slots: Vec<CodeStackSlot>,
    used_callee_saved: Vec<String>,
    stack_size: usize,
    next_register: usize,
    next_label: usize,
    trap: Option<TrapState>,
    loop_stack: Vec<LoopLabels>,
    active_cleanups: Vec<ActiveCleanup>,
    pending_result_slots: Option<PendingResultSlots>,
    error_arena_restore_slot: Option<usize>,
    /// When set, an inline built-in error return (`emit_error_register_return`)
    /// branches to this label instead of returning, leaving the raw `Result` in
    /// the standard tag/value/message registers. Used to make inline conversions
    /// (`toInt`, …) trappable: an inline `TRAP` materializes the raw `Result`
    /// rather than auto-propagating.
    raw_result_capture: Option<String>,
    /// Project-relative source file of the function currently being lowered. Used
    /// to build `ErrorLoc.filename` for errors that originate in this function.
    current_file: String,
    /// Source location of the error-originating node currently being lowered.
    /// Updated as `lower_value` descends into call/arithmetic nodes and consulted
    /// when an error is freshly created (overflow, divide-by-zero, helper failure)
    /// so its `ErrorLoc` records the true origin.
    current_loc: NirSourceLoc,
    /// Resource ownership decisions (escape analysis, §15.6) for the function
    /// being lowered, keyed by `RES` binding name. Drives where each resource's
    /// close obligation lives (its own scope, an outer collection's owned-list,
    /// or out via a returned collection).
    resource_owners: HashMap<String, crate::escape::ResOwner>,
    /// Collection binding names that own a runtime owned-list (some resource
    /// floats up to their scope).
    owner_collections: HashSet<String>,
    /// Live owned-lists: collection binding name -> head-pointer stack slot.
    owned_list_heads: HashMap<String, usize>,
}

#[derive(Clone)]
struct LocalValue {
    type_: String,
    stack_offset: usize,
    constant: Option<NirValue>,
}

#[derive(Clone)]
struct GlobalValue {
    type_: String,
    offset: usize,
}

#[derive(Clone)]
struct ValueResult {
    type_: String,
    location: String,
    text: String,
}

struct TrapState {
    name: String,
    label: String,
    in_trap_body: bool,
}

#[derive(Clone)]
struct LoopLabels {
    kind: crate::ast::LoopKind,
    continue_label: String,
    exit_label: String,
    cleanup_depth: usize,
}

#[derive(Clone)]
struct ThreadCleanup {
    name: String,
    symbol: String,
}

#[derive(Clone)]
struct ResourceCleanup {
    name: String,
    symbol: String,
}

#[derive(Clone)]
struct ResourceUnionCleanup {
    name: String,
    /// `(tag, close_symbol)` per active variant; drop reads the union tag and
    /// calls the matching close op on the variant's resource pointer.
    variants: Vec<(usize, String)>,
}

/// A per-scope runtime owned-list (§15.6): the close obligations for resources
/// whose ownership floated up to this scope from inner blocks. The list is a
/// nul-terminated singly linked list of `{record_ptr, next}` nodes, with its
/// head in `head_slot`; draining walks it head-first (most-recent first) and
/// closes each record once (the close is closed-flag idempotent).
#[derive(Clone)]
struct OwnedListCleanup {
    /// The owning collection binding's name (for transfer-on-return lookup).
    name: String,
    /// Stack offset of the list head pointer (0 when empty).
    head_slot: usize,
    /// Close op symbol for the collection's resource element type.
    close_symbol: String,
}

#[derive(Clone)]
enum ActiveCleanup {
    Thread(ThreadCleanup),
    Resource(ResourceCleanup),
    ResourceUnion(ResourceUnionCleanup),
    OwnedList(OwnedListCleanup),
}

#[derive(Clone, Copy)]
struct PendingResultSlots {
    value: usize,
    tag: usize,
    message: usize,
    // Stashed error-source pointer (`ErrorLoc`) for the pending error result.
    source: usize,
}

#[derive(Clone, Copy)]
enum ExitDestination {
    Return,
    Trap,
}

#[derive(Clone)]
struct PayloadSlot {
    slot: usize,
    type_: String,
}

#[derive(Clone)]
struct CollectionValueSlot {
    key: Option<PayloadSlot>,
    value: PayloadSlot,
}

struct CollectionTypeLayout {
    kind: usize,
    key_type_code: usize,
    value_type_code: usize,
}

#[derive(Clone)]
struct TypeModel {
    enum_members: HashMap<(String, String), usize>,
    record_fields: HashMap<String, Vec<(String, String)>>,
    union_names: HashSet<String>,
    union_variants: HashMap<String, String>,
    union_variant_unions: HashMap<String, HashSet<String>>,
    union_variant_tags: HashMap<String, usize>,
    union_variant_fields: HashMap<String, Vec<(String, String)>>,
}

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    packages: &[PathBuf],
    platform: &dyn CodegenPlatform,
) -> Result<NativeCodePlan, String> {
    if module.target != platform.target() {
        return Err(format!(
            "native code platform '{}' cannot lower module target '{}'",
            platform.target(),
            module.target
        ));
    }
    // Imported packages are now decoded and merged into the project IR upstream
    // (see `lower::lower_project`) and lowered as ordinary functions through this
    // same codegen. The legacy flat binary_repr -> native package bridge is no
    // longer used: there are no separate package exports to lower here.
    let _ = packages;
    let mut function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    for import in &module.imports {
        function_symbols.insert(import.name.clone(), import.symbol.clone());
    }
    let globals = module
        .globals
        .iter()
        .enumerate()
        .map(|(index, global)| {
            (
                global.name.clone(),
                GlobalValue {
                    type_: global.type_.clone(),
                    offset: ENTRY_GLOBALS_OFFSET + index * 8,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<_, _>>();
    let platform_imports = native_plan
        .platform_imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect::<HashMap<_, _>>();
    let imports = native_plan
        .platform_imports
        .iter()
        .map(|import| CodeImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect::<Vec<_>>();
    // Native `LINK` function return types (keyed `alias.func`) so calls used in
    // expressions resolve their result type (plan-linker.md §12).
    let mut package_return_types: HashMap<String, String> = HashMap::new();
    for function in &module.link_functions {
        package_return_types.insert(
            format!("{}.{}", function.alias, function.name),
            function.return_type.clone(),
        );
    }
    for import in &module.imports {
        // Re-export aliases route to a thunk; map the alias call name to the
        // thunk's return type too (plan-link-update.md §5a).
        if let Some(returns) = module
            .link_functions
            .iter()
            .find(|function| {
                nir::link_thunk_symbol(&function.alias, &function.name) == import.symbol
            })
            .map(|function| function.return_type.clone())
        {
            package_return_types
                .entry(import.name.clone())
                .or_insert(returns);
        }
    }
    let package_global_count = 0usize;
    let string_symbols = string_symbols(module);
    let mut string_objects = string_symbols.iter().collect::<Vec<_>>();
    string_objects.sort_by(|(_, left_symbol), (_, right_symbol)| left_symbol.cmp(right_symbol));
    let mut data_objects = string_objects
        .into_iter()
        .map(|(value, symbol)| CodeDataObject {
            symbol: symbol.clone(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: align(8 + value.len() + 1, 8),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    if module_requires_empty_string_constant(module) {
        data_objects.push(CodeDataObject {
            symbol: EMPTY_STRING_SYMBOL.to_string(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: 16,
            value: String::new(),
        });
    }
    // Writable global pointing at the main thread's arena state (set at startup,
    // read by `_mfb_shutdown` / the signal handler). Zero-initialized; it must
    // land in a writable section, which it does on every target that has an
    // entry: Linux entry programs are always dynamically linked (RW data
    // segment) and the macOS linker emits a writable `__DATA` segment.
    if module.entry.is_some() {
        data_objects.push(CodeDataObject {
            symbol: MAIN_ARENA_GLOBAL_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.main_arena.v1 { u64 arenaState }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        });
    }
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_fs_") || symbol.starts_with("_mfb_rt_thread_"))
    {
        for (_, message, symbol) in standard_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(CodeDataObject {
                    symbol: (*symbol).to_string(),
                    kind: "constant".to_string(),
                    layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"
                        .to_string(),
                    align: 8,
                    size: align(8 + message.len() + 1, 8),
                    value: (*message).to_string(),
                });
            }
        }
    }
    if module_uses_unicode_runtime_tables(module) {
        data_objects.extend(unicode_runtime_data_objects());
    }
    // `term::` console helpers reference fixed ANSI escape-sequence byte strings
    // (plan-01-term.md §6.1).
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_term_"))
    {
        data_objects.extend(term::console_data_objects());
    }
    // TLS helpers reference read-only C strings (library names + symbol names)
    // for their load-time dlopen/dlsym.
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        if platform.target().contains("macos") {
            data_objects.extend(tls::macos_tls_data_objects());
        } else {
            data_objects.extend(tls::tls_cstring_data_objects());
        }
    }
    let type_model = TypeModel::from_module_and_packages(module, packages)?;
    let mut code_functions = Vec::new();
    let mut runtime_symbols = native_plan.runtime_symbols.clone();
    let skip_entry_arena_destroy = platform.target() == "linux-aarch64"
        && runtime_symbols.iter().any(|symbol| {
            runtime::spec_for_symbol(symbol)
                .map(|spec| spec.call.starts_with("thread."))
                .unwrap_or(false)
        });

    let global_initializer_symbol = if module.globals.is_empty() {
        None
    } else {
        Some(nir::function_symbol(&nir::global_initializer_name(
            &module.project,
        )))
    };

    // Native `LINK` bindings reserve one writable global slot per function (just
    // past the program's own globals) for their dlopen/dlsym-resolved pointers
    // (plan-linker.md §12.1).
    let globals_base = module.globals.len() + package_global_count;
    let link_count = module.link_functions.len();
    // Each `FREE` block resolves its deallocator into an additional writable slot,
    // reserved just past the per-function slots (mfbasic.md §17).
    let free_count = module
        .link_functions
        .iter()
        .filter(|function| function.free.is_some())
        .count();
    let link_slot_count = link_count + free_count;
    // `term::` keeps its TUI-mode state (the §4.2.1 `active` gate plus current
    // attributes) in writable slots reserved just past the program globals and
    // `LINK` slots in the program-entry frame (plan-01-term.md §6.2). The slots
    // are addressed off the pinned arena-state register `x19`, so every console
    // helper reaches the state at a fixed offset without per-module threading
    // beyond this byte offset. When the program never uses `term::`, no slots are
    // reserved.
    let uses_term = runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_term_"));
    // Whether the program uses the `math::` random generator. When it does we
    // emit the PCG64 helpers, seed each thread's arena, and draw a fresh
    // per-thread stream on spawn.
    let uses_rng =
        module_uses_call(module, "math.rand") || module_uses_call(module, "math.seed");
    // The auto-restore on exit (§6.5) and a program that only calls `term::on()`
    // both need the `off` helper emitted; ensure it is present whenever `term::`
    // is used.
    if uses_term && !runtime_symbols.iter().any(|s| s == "_mfb_rt_term_term_off") {
        runtime_symbols.push("_mfb_rt_term_term_off".to_string());
    }
    let term_state_offset = if uses_term {
        Some(ENTRY_GLOBALS_OFFSET + (globals_base + link_slot_count) * 8)
    } else {
        None
    };
    let term_state_slots = if uses_term { TERM_STATE_SLOTS } else { 0 };
    let link_init_symbol = if link_count > 0 {
        Some(nir::LINK_INIT_SYMBOL)
    } else {
        None
    };
    // Install SIGINT/SIGTERM handlers for console programs only. App-mode builds
    // keep their window-driven finish path (the worker has no Ctrl-C semantics),
    // but still share `_mfb_shutdown` for their normal-exit cleanup.
    let register_signal_handlers =
        module.entry.is_some() && module.build_mode != crate::target::NativeBuildMode::MacApp;
    if let Some(entry) = &module.entry {
        let language_entry_symbol = nir::function_symbol(&entry.name);
        let entry_stack_size = align(
            ENTRY_STACK_SIZE + (globals_base + link_slot_count + term_state_slots) * 8,
            16,
        );
        let entry_global_slots = globals_base + link_slot_count + term_state_slots;
        if module.build_mode == crate::target::NativeBuildMode::MacApp {
            // App mode (plan-04-macos-app.md §6.6): the standard program entry runs
            // on a worker thread under `_mfb_macapp_program`, while `_main` becomes
            // the AppKit bootstrap that creates the window and spawns the worker.
            let app_spec = AppEntrySpec {
                language_entry_accepts_args: entry.accepts_args,
                uses_term,
            };
            let app_entry = platform.emit_app_program_entry(&app_spec, &platform_imports).ok_or_else(|| {
                format!(
                    "native target '{}' does not support app mode codegen",
                    platform.target()
                )
            })??;
            // The worker runs the standard program-entry logic under its own symbol.
            code_functions.push(lower_program_entry(
                MACAPP_PROGRAM_SYMBOL,
                &language_entry_symbol,
                &entry.returns,
                entry.accepts_args,
                global_initializer_symbol.as_deref(),
                link_init_symbol,
                entry_stack_size,
                entry_global_slots,
                &platform_imports,
                platform,
                module_may_record_cleanup_failure(module),
                uses_rng,
                register_signal_handlers,
            )?);
            code_functions.extend(app_entry);
            data_objects.extend(platform.app_mode_data_objects());
        } else {
            code_functions.push(lower_program_entry(
                "_main",
                &language_entry_symbol,
                &entry.returns,
                entry.accepts_args,
                global_initializer_symbol.as_deref(),
                link_init_symbol,
                entry_stack_size,
                entry_global_slots,
                &platform_imports,
                platform,
                module_may_record_cleanup_failure(module),
                uses_rng,
                register_signal_handlers,
            )?);
        }
    }
    for function in &module.functions {
        code_functions.push(lower_function(
            function,
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    for (name, type_, symbol) in builtin_function_refs(module) {
        code_functions.push(lower_builtin_function_wrapper(
            &name,
            &type_,
            &symbol,
            &function_symbols,
            &functions,
            &package_return_types,
            &platform_imports,
            &globals,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    code_functions.push(lower_arena_alloc(platform)?);
    code_functions.push(lower_arena_destroy(platform)?);
    if module.entry.is_some() {
        code_functions.push(lower_shutdown(uses_term, skip_entry_arena_destroy));
    }
    if register_signal_handlers {
        code_functions.push(lower_signal_handler(platform)?);
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_readBytes")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_readAllBytes")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_readAllBytes".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_writeTextAtomic")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_writeText")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_writeText".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_writeBytesAtomic")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_fs_fs_writeBytes")
    {
        runtime_symbols.push("_mfb_rt_fs_fs_writeBytes".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_receive")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_read")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_read".to_string());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_send")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_thread_thread_emit")
    {
        runtime_symbols.push("_mfb_rt_thread_thread_emit".to_string());
    }
    // The `connectTcp(Address)` overload routes to a paired helper that shares
    // `connectTcp`'s libc imports; emit it whenever `connectTcp` is present.
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_net_net_connectTcp")
        && !runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_net_net_connectTcpAddr")
    {
        runtime_symbols.push("_mfb_rt_net_net_connectTcpAddr".to_string());
    }
    // App-mode io.input composes io.write (prompt -> transcript) + io.readLine
    // (read the window input pipe), so ensure both helpers are emitted
    // (plan-04-macos-app.md §5.4).
    if module.build_mode == crate::target::NativeBuildMode::MacApp
        && runtime_symbols
            .iter()
            .any(|symbol| symbol == "_mfb_rt_io_io_input")
    {
        for dependency in ["_mfb_rt_io_io_write", "_mfb_rt_io_io_readLine"] {
            if !runtime_symbols.iter().any(|symbol| symbol == dependency) {
                runtime_symbols.push(dependency.to_string());
            }
        }
    }
    for symbol in &runtime_symbols {
        code_functions.push(lower_runtime_helper(
            symbol,
            module.build_mode,
            term_state_offset,
            uses_rng,
            &platform_imports,
            platform,
        )?);
    }
    if uses_rng {
        code_functions.push(lower_rng_next());
        code_functions.push(lower_rng_seed_at());
    }
    let link_returns_cstring = module.link_functions.iter().any(|function| {
        function.abi_return_name == "return"
            && function.abi_return_ctype == "CPtr"
            && function.return_type == "String"
    });
    if link_returns_cstring
        || runtime_symbols.iter().any(|symbol| {
            matches!(
                symbol.as_str(),
                "_mfb_rt_fs_fs_readText"
                    | "_mfb_rt_fs_fs_readAll"
                    | "_mfb_rt_fs_fs_readLine"
                    | "_mfb_rt_net_net_readText"
                    | "_mfb_rt_net_net_receiveTextFrom"
                    | "_mfb_rt_tls_tls_readText"
            )
        })
    {
        code_functions.push(lower_validate_utf8_helper());
    }
    // The macOS TLS backend bridges Network.framework's async callbacks to a
    // semaphore via small emitted block-invoke functions.
    if platform.target().contains("macos")
        && runtime_symbols
            .iter()
            .any(|symbol| symbol.starts_with("_mfb_rt_tls_"))
    {
        code_functions.extend(tls::macos_tls_aux_functions());
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_fs_fs_listDirectory")
    {
        code_functions.push(lower_sort_string_list_helper());
    }
    if module_uses_call(module, "fs.pathJoin") {
        code_functions.push(lower_fs_path_join_helper(platform));
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_start")
    {
        code_functions.push(lower_thread_trampoline(&platform_imports, platform)?);
    }

    // Native `LINK` marshaling thunks + load-time initializer (plan-linker.md §12).
    if link_count > 0 {
        let support = link_thunk::emit_link_support(
            &module.link_functions,
            globals_base,
            &platform_imports,
            platform,
        )?;
        code_functions.extend(support.functions);
        data_objects.extend(support.data_objects);
        for (_, message, symbol) in native_link_error_messages() {
            if !data_objects.iter().any(|object| object.symbol == *symbol) {
                data_objects.push(CodeDataObject {
                    symbol: symbol.to_string(),
                    kind: "constant".to_string(),
                    layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }"
                        .to_string(),
                    align: 8,
                    size: align(8 + message.len() + 1, 8),
                    value: message.to_string(),
                });
            }
        }
    }

    let plan = NativeCodePlan {
        target: module.target.clone(),
        build_mode: module.build_mode,
        arch: platform.arch().to_string(),
        project: module.project.clone(),
        entry_symbol: module.entry.as_ref().map(|_| "_main".to_string()),
        imports,
        data_objects,
        functions: code_functions,
    };
    plan.validate()?;
    Ok(plan)
}

impl NativeCodePlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.target.is_empty() {
            return Err("native code plan target must not be empty".to_string());
        }
        if self.arch.is_empty() {
            return Err("native code plan arch must not be empty".to_string());
        }
        if self.project.is_empty() {
            return Err("native code plan project name must not be empty".to_string());
        }
        if self.functions.is_empty() {
            return Err("native code plan requires at least one function".to_string());
        }
        if let Some(entry_symbol) = &self.entry_symbol {
            if !self
                .functions
                .iter()
                .any(|function| &function.symbol == entry_symbol)
            {
                return Err(format!(
                    "native code plan entry symbol '{entry_symbol}' does not resolve"
                ));
            }
        }
        let defined_symbols = self
            .functions
            .iter()
            .map(|function| function.symbol.clone())
            .collect::<Vec<_>>();
        let imported_symbols = self
            .imports
            .iter()
            .map(|import| import.symbol.clone())
            .collect::<Vec<_>>();
        for import in &self.imports {
            if import.library.is_empty() || import.symbol.is_empty() {
                return Err("native code plan contains an incomplete import".to_string());
            }
        }
        let data_symbols = self
            .data_objects
            .iter()
            .map(|object| object.symbol.clone())
            .collect::<Vec<_>>();
        for object in &self.data_objects {
            if object.symbol.is_empty() || object.kind.is_empty() || object.layout.is_empty() {
                return Err("native code plan contains an incomplete data object".to_string());
            }
            if object.align == 0 || object.size == 0 {
                return Err(format!(
                    "native code data object '{}' must have nonzero size and alignment",
                    object.symbol
                ));
            }
        }
        for function in &self.functions {
            function.validate(&defined_symbols, &imported_symbols, &data_symbols)?;
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-code-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"arch\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.arch),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.imports, 2),
            join_json(&self.data_objects, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl CodeFunction {
    fn validate(
        &self,
        defined_symbols: &[String],
        imported_symbols: &[String],
        data_symbols: &[String],
    ) -> Result<(), String> {
        if self.name.is_empty() || self.symbol.is_empty() {
            return Err("native code function name and symbol must not be empty".to_string());
        }
        if self.instructions.is_empty() {
            return Err(format!(
                "native code function '{}' has no instructions",
                self.name
            ));
        }
        if !self
            .instructions
            .iter()
            .any(|instruction| instruction.op == CodeOp::Ret)
        {
            return Err(format!(
                "native code function '{}' has no return instruction",
                self.name
            ));
        }
        for relocation in &self.relocations {
            if relocation.from != self.symbol {
                return Err(format!(
                    "native code relocation source '{}' does not match function '{}'",
                    relocation.from, self.symbol
                ));
            }
            match relocation.binding.as_str() {
                "internal" => {
                    if !defined_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code internal relocation target '{}' is not defined",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code internal relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                "external" => {
                    if !imported_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code external relocation target '{}' is not imported",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_none() {
                        return Err(format!(
                            "native code external relocation '{}' must name a library",
                            relocation.to
                        ));
                    }
                }
                "data" => {
                    if !data_symbols.contains(&relocation.to)
                        && !defined_symbols.contains(&relocation.to)
                    {
                        return Err(format!(
                            "native code data relocation target '{}' is not a data object or defined symbol",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code data relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "native code relocation '{}' has invalid binding '{}'",
                        relocation.to, other
                    ));
                }
            }
        }
        for instruction in &self.instructions {
            instruction.validate()?;
        }
        Ok(())
    }
}

impl TypeModel {
    fn empty() -> Self {
        Self {
            enum_members: HashMap::new(),
            record_fields: HashMap::new(),
            union_names: HashSet::new(),
            union_variants: HashMap::new(),
            union_variant_unions: HashMap::new(),
            union_variant_tags: HashMap::new(),
            union_variant_fields: HashMap::new(),
        }
    }

    fn from_module(module: &NirModule) -> Result<Self, String> {
        let mut enum_members = HashMap::new();
        let mut record_fields = HashMap::new();
        let mut union_names = HashSet::new();
        let mut union_variants = HashMap::new();
        let mut union_variant_unions = HashMap::<String, HashSet<String>>::new();
        let mut union_variant_tags = HashMap::new();
        let mut union_variant_fields = HashMap::new();
        for type_ in &module.types {
            match type_.kind.as_str() {
                "type" | "record" => {
                    record_fields.insert(
                        type_.name.clone(),
                        type_
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.type_.clone()))
                            .collect(),
                    );
                }
                "enum" => {
                    for (index, member) in type_.members.iter().enumerate() {
                        enum_members.insert((type_.name.clone(), member.name.clone()), index);
                    }
                }
                "union" => {
                    union_names.insert(type_.name.clone());
                    for (index, variant) in expanded_nir_union_variants(module, &type_.name)
                        .iter()
                        .enumerate()
                    {
                        union_variants
                            .entry(variant.name.clone())
                            .or_insert_with(|| type_.name.clone());
                        union_variant_unions
                            .entry(variant.name.clone())
                            .or_default()
                            .insert(type_.name.clone());
                        union_variant_tags.insert(variant.name.clone(), index);
                        union_variant_fields.insert(
                            variant.name.clone(),
                            variant
                                .fields
                                .iter()
                                .map(|field| (field.name.clone(), field.type_.clone()))
                                .collect(),
                        );
                    }
                }
                "resource" => {}
                other => {
                    return Err(format!(
                        "native code plan does not know type kind '{other}'"
                    ));
                }
            }
        }
        for type_name in ["Address", "Datagram", "DatagramText"] {
            if let Some(fields) = builtins::net::builtin_type_fields(type_name) {
                record_fields.insert(
                    type_name.to_string(),
                    fields
                        .iter()
                        .map(|(name, type_)| ((*name).to_string(), (*type_).to_string()))
                        .collect(),
                );
            }
        }
        for type_name in ["TermColor", "TermSize"] {
            if let Some(fields) = builtins::term::builtin_type_fields(type_name) {
                record_fields.insert(
                    type_name.to_string(),
                    fields
                        .iter()
                        .map(|(name, type_)| ((*name).to_string(), (*type_).to_string()))
                        .collect(),
                );
            }
        }
        // `Error` and `ErrorLoc` are read-only compiler/runtime records laid out
        // as ordinary 3-field records so construction, field access, copying, and
        // cleanup reuse the generic record machinery.
        record_fields.insert(
            "Error".to_string(),
            vec![
                ("code".to_string(), "Integer".to_string()),
                ("message".to_string(), "String".to_string()),
                ("source".to_string(), "ErrorLoc".to_string()),
            ],
        );
        record_fields.insert(
            "ErrorLoc".to_string(),
            vec![
                ("filename".to_string(), "String".to_string()),
                ("line".to_string(), "Integer".to_string()),
                ("char".to_string(), "Integer".to_string()),
            ],
        );
        Ok(Self {
            enum_members,
            record_fields,
            union_names,
            union_variants,
            union_variant_unions,
            union_variant_tags,
            union_variant_fields,
        })
    }

    fn from_module_and_packages(module: &NirModule, packages: &[PathBuf]) -> Result<Self, String> {
        let mut model = Self::from_module(module)?;
        for package in packages {
            // A native `LINK` resource is exported as a zero-field opaque type for
            // naming, but its runtime value is a raw `CPtr` scalar handle — never a
            // record. Registering it as a record would make the backend copy it by
            // value on bind/return (an empty copy that loses the handle), so skip
            // native resource type exports and let them default to 8-byte scalars
            // (plan-linker.md §12, plan-link-update.md §10).
            let native_resources: HashSet<String> = binary_repr::read_package_resources(package)?
                .into_iter()
                .filter(|resource| resource.native)
                .map(|resource| resource.type_name)
                .collect();
            for type_export in binary_repr::read_package_type_exports(package)? {
                if native_resources.contains(&type_export.name) {
                    continue;
                }
                model.add_package_type_export(type_export);
            }
        }
        Ok(model)
    }

    fn add_package_type_export(&mut self, type_export: binary_repr::BinaryReprTypeExport) {
        match type_export.kind {
            binary_repr::BinaryReprExportKind::Type => {
                self.record_fields.insert(
                    type_export.name,
                    type_export
                        .fields
                        .into_iter()
                        .map(|field| (field.name, field.type_))
                        .collect(),
                );
            }
            binary_repr::BinaryReprExportKind::Enum => {
                for (index, member) in type_export.members.into_iter().enumerate() {
                    self.enum_members
                        .insert((type_export.name.clone(), member), index);
                }
            }
            binary_repr::BinaryReprExportKind::Union => {
                self.union_names.insert(type_export.name.clone());
                for (index, variant) in type_export.variants.into_iter().enumerate() {
                    self.union_variants
                        .entry(variant.name.clone())
                        .or_insert_with(|| type_export.name.clone());
                    self.union_variant_unions
                        .entry(variant.name.clone())
                        .or_default()
                        .insert(type_export.name.clone());
                    self.union_variant_tags.insert(variant.name.clone(), index);
                    self.union_variant_fields.insert(
                        variant.name,
                        variant
                            .fields
                            .into_iter()
                            .map(|field| (field.name, field.type_))
                            .collect(),
                    );
                }
            }
            binary_repr::BinaryReprExportKind::Func | binary_repr::BinaryReprExportKind::Sub => {}
        }
    }

    fn variants_for_union<'a>(&'a self, union: &'a str) -> impl Iterator<Item = &'a String> + 'a {
        self.union_variant_unions
            .iter()
            .filter(move |(_, unions)| unions.contains(union))
            .map(|(variant, _)| variant)
    }
}

fn expanded_nir_union_variants<'a>(
    module: &'a NirModule,
    union_name: &str,
) -> Vec<&'a super::nir::NirVariant> {
    let Some(type_) = module
        .types
        .iter()
        .find(|candidate| candidate.kind == "union" && candidate.name == union_name)
    else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    for include in &type_.includes {
        variants.extend(expanded_nir_union_variants(module, include));
    }
    variants.extend(type_.variants.iter());
    variants
}

#[allow(clippy::too_many_arguments)]
fn lower_program_entry(
    entry_symbol: &str,
    language_entry_symbol: &str,
    language_entry_returns: &str,
    language_entry_accepts_args: bool,
    global_initializer_symbol: Option<&str>,
    link_init_symbol: Option<&str>,
    entry_stack_size: usize,
    global_slot_count: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    emit_cleanup_failure_audit: bool,
    seed_rng: bool,
    register_signal_handlers: bool,
) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(entry_stack_size),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 8),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 16),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 24),
    ];
    if emit_cleanup_failure_audit {
        instructions.extend([
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_CODE_OFFSET,
            ),
            abi::store_u64(
                "x31",
                ARENA_STATE_REGISTER,
                ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
            ),
        ]);
    }
    for index in 0..global_slot_count {
        instructions.push(abi::store_u64(
            "x31",
            ARENA_STATE_REGISTER,
            ENTRY_GLOBALS_OFFSET + index * 8,
        ));
    }
    let mut relocations = Vec::new();
    let error_label = "entry_error";
    let exit_label = "entry_exit";
    // Publish this thread's arena-state address to the writable global so the
    // signal handler and `_mfb_shutdown` can find the arena without `x19`. `x9`
    // is a scratch temporary here; `x0`/`x1` (argc/argv) are left untouched.
    push_symbol_address(
        entry_symbol,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::store_u64(ARENA_STATE_REGISTER, "x9", 0));
    // Install SIGINT/SIGTERM handlers (console programs). `signal()` clobbers
    // `x0`/`x1`, so argc/argv are parked below the frame across the calls; `x19`
    // pins the entry frame, so temporarily lowering `sp` is safe.
    if register_signal_handlers {
        instructions.extend([
            abi::subtract_stack(16),
            abi::store_u64("x0", abi::stack_pointer(), 0),
            abi::store_u64("x1", abi::stack_pointer(), 8),
        ]);
        for signo in ["2", "15"] {
            instructions.push(abi::move_immediate("x0", "Integer", signo));
            push_symbol_address(
                entry_symbol,
                SIGNAL_HANDLER_SYMBOL,
                "x1",
                &mut instructions,
                &mut relocations,
            );
            platform.emit_libc_call(
                "signal",
                entry_symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        instructions.extend([
            abi::load_u64("x0", abi::stack_pointer(), 0),
            abi::load_u64("x1", abi::stack_pointer(), 8),
            abi::add_stack(16),
        ]);
    }
    // Seed this thread's PCG64 generator from the OS entropy pool before any
    // user code (including global initializers, which may call `math::rand`).
    // The seed scratch lives in the as-yet-unused args slot; pre-fill it with
    // the arena address so a `getentropy` failure still yields a varying seed.
    if seed_rng {
        instructions.extend([
            abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::add_immediate(abi::return_register(), abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        platform.emit_random_bytes(
            entry_symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::load_u64("x1", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::move_register(abi::return_register(), ARENA_STATE_REGISTER),
            abi::branch_link(RNG_SEED_SYMBOL),
        ]);
        relocations.push(internal_branch(entry_symbol, RNG_SEED_SYMBOL));
    }
    // Resolve native `LINK` bindings (dlopen/dlsym) before anything runs; a load
    // failure aborts before `main` through the standard error path
    // (plan-linker.md §12.1).
    if let Some(symbol) = link_init_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if let Some(symbol) = global_initializer_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: entry_symbol.to_string(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
            abi::branch_ne("global_initializer_not_program_exit"),
            abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
            abi::branch(exit_label),
            abi::label("global_initializer_not_program_exit"),
        ]);
        instructions.extend([
            abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
            abi::branch_ne(error_label),
        ]);
    }
    if language_entry_accepts_args {
        instructions.extend([
            abi::store_u64("x0", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
            abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        ]);
        emit_entry_args_list_materialization(error_label, &mut instructions, &mut relocations);
        instructions.push(abi::load_u64(
            "x0",
            abi::stack_pointer(),
            ENTRY_ARGS_LIST_OFFSET,
        ));
    }
    instructions.push(abi::branch_link(language_entry_symbol));
    relocations.push(CodeRelocation {
        from: entry_symbol.to_string(),
        to: language_entry_symbol.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_PROGRAM_EXIT_TAG),
        abi::branch_ne("entry_not_program_exit"),
        abi::move_register(abi::return_register(), RESULT_VALUE_REGISTER),
        abi::branch(exit_label),
        abi::label("entry_not_program_exit"),
    ]);
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(error_label),
    ]);
    if language_entry_returns == "Nothing" {
        instructions.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    } else {
        instructions.push(abi::move_register(
            abi::return_register(),
            RESULT_VALUE_REGISTER,
        ));
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "255"),
            abi::branch_hi("entry_exit_range_error"),
        ]);
    }
    instructions.push(abi::branch(exit_label));
    if language_entry_returns == "Integer" {
        instructions.extend([
            abi::label("entry_exit_range_error"),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OVERFLOW_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            entry_symbol,
            ERR_OVERFLOW_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(error_label));
    }
    instructions.extend([
        abi::label(error_label),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 32),
        abi::move_register("x20", RESULT_ERROR_MESSAGE_REGISTER),
    ]);
    emit_write_string_object(
        ENTRY_ERROR_PREFIX_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_integer_to_stderr(
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_SEPARATOR_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        entry_symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        entry_symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    if emit_cleanup_failure_audit {
        emit_cleanup_failure_audit_report(
            entry_symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        "255",
    ));
    instructions.push(abi::label(exit_label));
    // Run the shared teardown (terminal restore + arena free), then exit. The
    // exit code is parked in the arena-state scratch slot across the call: that
    // slot lives in this stack-resident entry frame (not in the freed mmap
    // blocks), and `_mfb_shutdown` preserves `x19`, so it is valid on return.
    // `_mfb_shutdown` is internally gated and idempotent, so the SIGINT/SIGTERM
    // handler racing this path cannot double-free.
    instructions.push(abi::store_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    instructions.push(abi::branch_link(SHUTDOWN_SYMBOL));
    relocations.push(internal_branch(entry_symbol, SHUTDOWN_SYMBOL));
    instructions.push(abi::load_u64(
        abi::return_register(),
        ARENA_STATE_REGISTER,
        32,
    ));
    platform.emit_program_exit(entry_symbol, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: if entry_symbol == "_main" {
            "program.entry".to_string()
        } else {
            "program.entry.macapp".to_string()
        },
        symbol: entry_symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

fn emit_entry_args_list_materialization(
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGC_OFFSET),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x22", "Integer", "0"),
        abi::move_immediate("x23", "Integer", "0"),
        abi::label("entry_args_count_loop"),
        abi::compare_registers("x23", "x20"),
        abi::branch_eq("entry_args_count_done"),
        abi::load_u64("x24", "x21", 0),
        abi::move_register("x25", "x24"),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_count_len_loop"),
        abi::load_u8("x27", "x25", 0),
        abi::compare_immediate("x27", "0"),
        abi::branch_eq("entry_args_count_len_done"),
        abi::add_immediate("x26", "x26", 1),
        abi::add_immediate("x25", "x25", 1),
        abi::branch("entry_args_count_len_loop"),
        abi::label("entry_args_count_len_done"),
        abi::add_registers("x22", "x22", "x26"),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x23", "x23", 1),
        abi::branch("entry_args_count_loop"),
        abi::label("entry_args_count_done"),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x25", "x25", "x22"),
        abi::store_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::store_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::add_immediate(abi::return_register(), "x25", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: "_main".to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("entry_args_alloc_ok"),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address("_main", ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(error_label),
        abi::label("entry_args_alloc_ok"),
        abi::store_u64("x1", abi::stack_pointer(), ENTRY_ARGS_LIST_OFFSET),
        abi::load_u64("x22", abi::stack_pointer(), ENTRY_ARGS_DATA_LENGTH_OFFSET),
        abi::load_u64("x20", abi::stack_pointer(), ENTRY_ARGS_COUNT_SAVED_OFFSET),
        abi::move_immediate("x8", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x8", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Byte", "1"),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x20", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x22", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x23", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x24", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x25", "x20", "x24"),
        abi::add_registers("x24", "x23", "x25"),
        abi::move_immediate("x25", "Integer", "0"),
        abi::load_u64("x21", abi::stack_pointer(), ENTRY_ARGV_OFFSET),
        abi::move_immediate("x26", "Integer", "0"),
        abi::label("entry_args_fill_loop"),
        abi::compare_registers("x26", "x20"),
        abi::branch_eq("entry_args_fill_done"),
        abi::load_u64("x27", "x21", 0),
        abi::move_register("x28", "x27"),
        abi::move_immediate("x9", "Integer", "0"),
        abi::label("entry_args_fill_len_loop"),
        abi::load_u8("x10", "x28", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq("entry_args_fill_len_done"),
        abi::add_immediate("x9", "x9", 1),
        abi::add_immediate("x28", "x28", 1),
        abi::branch("entry_args_fill_len_loop"),
        abi::label("entry_args_fill_len_done"),
        abi::move_immediate("x11", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x11", "x23", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x23", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x25", "x23", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::store_u64("x9", "x23", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::move_immediate("x11", "Integer", "0"),
        abi::label("entry_args_copy_loop"),
        abi::compare_registers("x11", "x9"),
        abi::branch_eq("entry_args_copy_done"),
        abi::load_u8("x12", "x27", 0),
        abi::store_u8("x12", "x24", 0),
        abi::add_immediate("x27", "x27", 1),
        abi::add_immediate("x24", "x24", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::branch("entry_args_copy_loop"),
        abi::label("entry_args_copy_done"),
        abi::add_registers("x25", "x25", "x9"),
        abi::add_immediate("x23", "x23", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x21", "x21", 8),
        abi::add_immediate("x26", "x26", 1),
        abi::branch("entry_args_fill_loop"),
        abi::label("entry_args_fill_done"),
    ]);
}

fn emit_cleanup_failure_audit_report(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let done = "entry_cleanup_failure_audit_done";
    instructions.extend([
        abi::load_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(done),
    ]);
    emit_write_string_object(
        CLEANUP_FAILURE_PREFIX_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::load_u64(
        "x9",
        ARENA_STATE_REGISTER,
        ARENA_CLEANUP_FAILURE_CODE_OFFSET,
    ));
    instructions.push(abi::store_u64("x9", ARENA_STATE_REGISTER, 32));
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_cleanup_failure_code",
    )?;
    emit_write_string_object(
        CLEANUP_FAILURE_SEPARATOR_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(
            "x20",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
    )?;
    instructions.push(abi::label(done));
    Ok(())
}

fn lower_arena_alloc(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate("x9", "x1", 1),
        abi::and_registers("x10", "x1", "x9"),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne("arena_alloc_invalid"),
        abi::compare_immediate("x0", "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate("x0", "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::move_register("x20", "x0"),
        abi::move_register("x21", "x1"),
        abi::label("arena_alloc_try_current"),
        abi::load_u64("x22", ARENA_STATE_REGISTER, 0),
        abi::compare_immediate("x22", "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64("x23", "x22", 16),
        abi::load_u64("x24", "x22", 24),
        abi::add_immediate("x25", "x22", ARENA_BLOCK_HEADER_SIZE),
        abi::add_registers("x26", "x25", "x24"),
        abi::compare_registers("x26", "x25"),
        abi::branch_lo("arena_alloc_oom"),
        abi::subtract_immediate("x27", "x21", 1),
        abi::move_register("x15", "x26"),
        abi::add_registers("x26", "x26", "x27"),
        abi::compare_registers("x26", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::bitwise_not("x27", "x27"),
        abi::and_registers("x26", "x26", "x27"),
        abi::add_registers("x28", "x26", "x20"),
        abi::compare_registers("x28", "x26"),
        abi::branch_lo("arena_alloc_oom"),
        abi::subtract_registers("x28", "x28", "x25"),
        abi::compare_registers("x28", "x23"),
        abi::branch_hi("arena_alloc_grow"),
        abi::store_u64("x28", "x22", 24),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", "x26"),
        abi::return_(),
        abi::label("arena_alloc_grow"),
        abi::add_registers("x23", "x20", "x21"),
        abi::compare_registers("x23", "x20"),
        abi::branch_lo("arena_alloc_oom"),
        abi::add_immediate("x23", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::move_immediate("x14", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::compare_registers("x23", "x14"),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate("x23", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register("x15", "x23"),
        abi::add_immediate("x23", "x23", 4095),
        abi::compare_registers("x23", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate("x24", "Integer", &(!4095_u64).to_string()),
        abi::and_registers("x23", "x23", "x24"),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    platform.emit_arena_map(&mut instructions)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
        abi::load_u64("x24", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x24", abi::return_register(), 0),
        abi::store_u64("x23", abi::return_register(), 8),
        abi::subtract_immediate("x24", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64("x24", abi::return_register(), 16),
        abi::store_u64("x31", abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        abi::branch("arena_alloc_try_current"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::return_(),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_alloc".to_string(),
        symbol: ARENA_ALLOC_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::load_u64("x20", ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate("x20", "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64("x21", "x20", 0),
        abi::load_u64("x1", "x20", 8),
        abi::move_register(abi::return_register(), "x20"),
    ]);
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register("x20", "x21"),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_destroy".to_string(),
        symbol: ARENA_DESTROY_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

/// Shared process teardown. Reads the main arena-state address from the writable
/// global, clears the global (so a second entry — e.g. a signal arriving during
/// normal cleanup — becomes a no-op), pins it in `x19`, then conditionally
/// restores the terminal and frees the arena. Both underlying helpers are
/// idempotent (`term::off` gates on its `active` flag; `arena_destroy` clears the
/// block-list head), so the guard is belt-and-suspenders. Preserves `x19`/`x30`
/// for its callers (the entry exit path relies on `x19` afterwards).
fn lower_shutdown(auto_term_off: bool, skip_arena_destroy: bool) -> CodeFunction {
    let done = "shutdown_done";
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
    ];
    let mut relocations = Vec::new();
    push_symbol_address(
        SHUTDOWN_SYMBOL,
        MAIN_ARENA_GLOBAL_SYMBOL,
        "x9",
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64("x10", "x9", 0),
        abi::store_u64("x31", "x9", 0),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(done),
        abi::move_register(ARENA_STATE_REGISTER, "x10"),
    ]);
    if auto_term_off {
        instructions.push(abi::branch_link("_mfb_rt_term_term_off"));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, "_mfb_rt_term_term_off"));
    }
    if !skip_arena_destroy {
        instructions.push(abi::branch_link(ARENA_DESTROY_SYMBOL));
        relocations.push(internal_branch(SHUTDOWN_SYMBOL, ARENA_DESTROY_SYMBOL));
    }
    instructions.extend([
        abi::label(done),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(16),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.shutdown".to_string(),
        symbol: SHUTDOWN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// `void handler(int signo)` for SIGINT/SIGTERM: run the shared teardown, then
/// `_exit(128 + signo)`. It never returns, so it need not preserve the
/// interrupted context; it locates the arena through `_mfb_shutdown`'s global
/// read rather than the interrupted `x19`. The 16-byte frame keeps `sp` aligned
/// across the `bl`s (Darwin requires this) and parks `signo` across the call.
fn lower_signal_handler(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::branch_link(SHUTDOWN_SYMBOL),
    ];
    let mut relocations = vec![internal_branch(SIGNAL_HANDLER_SYMBOL, SHUTDOWN_SYMBOL)];
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 128),
        abi::add_stack(16),
    ]);
    platform.emit_program_exit(SIGNAL_HANDLER_SYMBOL, &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: "runtime.signal_handler".to_string(),
        symbol: SIGNAL_HANDLER_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

/// Append the PCG64 LCG step `state = state * MULT + INC` operating on the
/// 128-bit state held in (`lo`, `hi`). The limbs are read at the start and
/// rewritten in place; `x11`-`x16` are used as scratch (caller-saved, so these
/// leaf helpers need not preserve them).
fn emit_pcg_step(instructions: &mut Vec<CodeInstruction>, lo: &str, hi: &str) {
    instructions.extend([
        // 128-bit (truncated) product of state by the 128-bit multiplier.
        abi::move_immediate("x11", "Integer", &PCG_MULT_LO.to_string()),
        abi::multiply_registers("x13", "x11", lo), // result low limb
        abi::unsigned_multiply_high_registers("x14", "x11", lo), // carry into high
        abi::multiply_registers("x15", "x11", hi), // MULT_LO * state_hi
        abi::move_immediate("x12", "Integer", &PCG_MULT_HI.to_string()),
        abi::multiply_registers("x16", "x12", lo), // MULT_HI * state_lo
        abi::add_registers("x14", "x14", "x15"),
        abi::add_registers("x14", "x14", "x16"), // result high limb
        // Add the 128-bit increment with carry between limbs.
        abi::move_immediate("x11", "Integer", &PCG_INC_LO.to_string()),
        abi::move_immediate("x12", "Integer", &PCG_INC_HI.to_string()),
        abi::add_registers_set_flags(lo, "x13", "x11"),
        abi::add_with_carry_registers(hi, "x14", "x12"),
    ]);
}

/// `_mfb_rng_next` — advance the calling thread's PCG64 generator one step and
/// return the next 64-bit value in `x0`. State lives in the arena (`x19`).
fn lower_rng_next() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_RNG_STATE_LO_OFFSET),
        abi::load_u64("x10", ARENA_STATE_REGISTER, ARENA_RNG_STATE_HI_OFFSET),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", ARENA_STATE_REGISTER, ARENA_RNG_STATE_LO_OFFSET),
        abi::store_u64("x10", ARENA_STATE_REGISTER, ARENA_RNG_STATE_HI_OFFSET),
        // XSL-RR output: rotate (hi ^ lo) right by the top 6 bits of hi.
        abi::shift_right_immediate("x11", "x10", 58),
        abi::exclusive_or_registers("x12", "x10", "x9"),
        abi::rotate_right_registers(abi::return_register(), "x12", "x11"),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.rng_next".to_string(),
        symbol: RNG_NEXT_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// `_mfb_rng_seed_at(x0 = arena ptr, x1 = seed)` — initialize the PCG64 state at
/// the given arena from a 64-bit seed, following the canonical seeding dance
/// (`state = 0; step; state += seed; step`).
fn lower_rng_seed_at() -> CodeFunction {
    let mut instructions = vec![abi::label("entry")];
    instructions.extend([
        abi::move_immediate("x9", "Integer", "0"),
        abi::move_immediate("x10", "Integer", "0"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::add_registers_set_flags("x9", "x9", "x1"),
        abi::add_with_carry_registers("x10", "x10", "xzr"),
    ]);
    emit_pcg_step(&mut instructions, "x9", "x10");
    instructions.extend([
        abi::store_u64("x9", "x0", ARENA_RNG_STATE_LO_OFFSET),
        abi::store_u64("x10", "x0", ARENA_RNG_STATE_HI_OFFSET),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.rng_seed_at".to_string(),
        symbol: RNG_SEED_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

fn emit_write_string_object(
    symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_page_address("x21", symbol),
        abi::add_page_offset("x21", "x21", symbol),
        abi::load_u64(abi::string_length_register(), "x21", 0),
        abi::add_immediate(abi::string_data_register(), "x21", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

fn emit_write_integer_to_stderr(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_write_integer_to_stderr_with_labels(
        from,
        platform_imports,
        platform,
        instructions,
        relocations,
        "entry_error_code",
    )
}

fn emit_write_integer_to_stderr_with_labels(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    label_prefix: &str,
) -> Result<(), String> {
    let absolute_ready_label = format!("{label_prefix}_absolute_ready");
    let digit_loop_label = format!("{label_prefix}_digit_loop");
    let digits_done_label = format!("{label_prefix}_digits_done");
    let write_label = format!("{label_prefix}_write");
    instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64("x21", ARENA_STATE_REGISTER, 32),
        abi::compare_immediate("x21", "0"),
        abi::branch_ge(&absolute_ready_label),
        abi::move_immediate("x22", "Integer", "0"),
        abi::subtract_registers("x21", "x22", "x21"),
        abi::label(&absolute_ready_label),
        abi::add_immediate("x23", abi::stack_pointer(), 64),
        abi::move_immediate("x24", "Integer", "10"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "48"),
        abi::store_u8("x22", "x23", 0),
        abi::branch(&digits_done_label),
        abi::label(&digit_loop_label),
        abi::unsigned_divide_registers("x25", "x21", "x24"),
        abi::multiply_subtract_registers("x26", "x25", "x24", "x21"),
        abi::add_immediate("x26", "x26", 48),
        abi::subtract_immediate("x23", "x23", 1),
        abi::store_u8("x26", "x23", 0),
        abi::move_register("x21", "x25"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(&digit_loop_label),
        abi::label(&digits_done_label),
        abi::compare_immediate("x19", "0"),
        abi::branch_ge(&write_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "45"),
        abi::store_u8("x22", "x23", 0),
        abi::label(&write_label),
        abi::add_immediate("x27", abi::stack_pointer(), 64),
        abi::subtract_registers(abi::string_length_register(), "x27", "x23"),
        abi::move_register(abi::string_data_register(), "x23"),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    instructions.push(abi::add_stack(64));
    Ok(())
}

fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, String>,
    platform_imports: &HashMap<String, String>,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let location = abi::argument_register(index)?;
            Ok(CodeParam {
                name: param.name.clone(),
                type_: param.type_.clone(),
                location,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut builder = CodeBuilder {
        current_symbol: nir::function_symbol(&function.name),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        current_file: function.file.clone(),
        current_loc: NirSourceLoc::default(),
        owner_collections: function
            .resource_owners
            .values()
            .filter_map(|owner| match owner {
                crate::escape::ResOwner::Float(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        resource_owners: function.resource_owners.clone(),
        owned_list_heads: HashMap::new(),
    };
    for param in &params {
        let stack_offset = builder.allocate_stack_object(&param.name, 8);
        builder.locals.insert(
            param.name.clone(),
            LocalValue {
                type_: param.type_.clone(),
                stack_offset,
                constant: None,
            },
        );
        builder.emit(abi::store_u64(
            &param.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        if CodeBuilder::is_thread_type(&param.type_) {
            builder
                .active_cleanups
                .push(ActiveCleanup::Thread(ThreadCleanup {
                    name: param.name.clone(),
                    symbol: CodeBuilder::thread_drop_symbol(),
                }));
        }
    }
    if let Some(name) = function.body.iter().find_map(|op| match op {
        NirOp::Trap { name, .. } => Some(name.clone()),
        _ => None,
    }) {
        let stack_offset = builder.allocate_stack_object(&name, 8);
        builder.locals.insert(
            name.clone(),
            LocalValue {
                type_: "Error".to_string(),
                stack_offset,
                constant: None,
            },
        );
        let label = builder.label("trap");
        builder.trap = Some(TrapState {
            name,
            label,
            in_trap_body: false,
        });
    }
    builder.lower_ops(&function.body)?;
    if !builder.current_block_returns() {
        builder.emit_return_exit(None)?;
    }
    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: function.name.clone(),
        symbol: nir::function_symbol(&function.name),
        params,
        returns: function.returns.clone(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

fn lower_builtin_function_wrapper(
    name: &str,
    type_: &str,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, String>,
    platform_imports: &HashMap<String, String>,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let (params, returns) = function_type_parts(type_).ok_or_else(|| {
        format!("native built-in function wrapper has malformed function type '{type_}'")
    })?;
    if params.len() != 1 || returns != "Boolean" {
        return Err(format!(
            "native built-in function wrapper expects a unary Boolean function, got '{type_}'"
        ));
    }

    let param = CodeParam {
        name: "value".to_string(),
        type_: params[0].clone(),
        location: abi::argument_register(0)?,
    };
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_collections: HashSet::new(),
        owned_list_heads: HashMap::new(),
    };

    let stack_offset = builder.allocate_stack_object("value", 8);
    builder.locals.insert(
        "value".to_string(),
        LocalValue {
            type_: param.type_.clone(),
            stack_offset,
            constant: None,
        },
    );
    builder.emit(abi::store_u64(
        &param.location,
        abi::stack_pointer(),
        stack_offset,
    ));

    let result = builder.lower_value(&NirValue::Call {
        target: name.to_string(),
        args: vec![NirValue::Local("value".to_string())],
        loc: NirSourceLoc::default(),
    })?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("builtin.{name}.{type_}"),
        symbol: symbol.to_string(),
        params: vec![param],
        returns: returns,
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

fn module_requires_empty_string_constant(module: &NirModule) -> bool {
    let type_model = TypeModel::from_module(module).unwrap_or_else(|_| TypeModel::empty());
    module.functions.iter().any(|function| {
        function
            .body
            .iter()
            .any(|op| op_requires_empty_string_constant(op, &type_model))
    })
}

fn op_requires_empty_string_constant(op: &NirOp, type_model: &TypeModel) -> bool {
    match op {
        NirOp::Bind {
            type_, value: None, ..
        } => type_requires_empty_string_constant(type_, type_model, &mut HashSet::new()),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => {
            then_body
                .iter()
                .any(|op| op_requires_empty_string_constant(op, type_model))
                || else_body
                    .iter()
                    .any(|op| op_requires_empty_string_constant(op, type_model))
        }
        NirOp::Match { cases, .. } => cases.iter().any(|case| {
            case.body
                .iter()
                .any(|op| op_requires_empty_string_constant(op, type_model))
        }),
        NirOp::While { body, .. } | NirOp::ForEach { body, .. } | NirOp::Trap { body, .. } => body
            .iter()
            .any(|op| op_requires_empty_string_constant(op, type_model)),
        _ => false,
    }
}

fn type_requires_empty_string_constant(
    type_: &str,
    type_model: &TypeModel,
    seen: &mut HashSet<String>,
) -> bool {
    if type_ == "String" {
        return true;
    }
    let Some(fields) = type_model.record_fields.get(type_) else {
        return false;
    };
    if !seen.insert(type_.to_string()) {
        return false;
    }
    let result = fields
        .iter()
        .any(|(_, field_type)| type_requires_empty_string_constant(field_type, type_model, seen));
    seen.remove(type_);
    result
}

fn lower_runtime_helper(
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    term_state_offset: Option<usize>,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    let app_mode = build_mode == crate::target::NativeBuildMode::MacApp;
    if builtins::term::is_term_call(spec.call) {
        let term_state_offset = term_state_offset.ok_or_else(|| {
            format!("native code plan emits '{symbol}' without reserving term state")
        })?;
        // App mode drives the synthesized TermView surface for the mode toggle
        // (plan-01-term.md §6.3); the remaining term:: helpers keep the shared
        // console backend until Phase 5 wires their app bodies.
        let app_term_helper = if app_mode {
            platform.emit_app_term_helper(spec.call, symbol, term_state_offset)
        } else {
            None
        };
        let (frame, instructions, relocations) = match app_term_helper {
            Some(result) => result?,
            None => {
                term::lower_term_helper(spec.call, symbol, term_state_offset, platform_imports, platform)?
            }
        };
        return Ok(CodeFunction {
            name: format!("runtime.{}", spec.call),
            symbol: symbol.to_string(),
            params: spec
                .abi
                .params
                .iter()
                .map(|param| CodeParam {
                    name: param.name.to_string(),
                    type_: param.type_.to_string(),
                    location: param.location.to_string(),
                })
                .collect(),
            returns: spec.abi.returns.to_string(),
            frame,
            stack_slots: Vec::new(),
            instructions,
            relocations,
        });
    }
    match spec.call {
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let stderr = matches!(spec.call, "io.printError" | "io.writeError");
            let newline = matches!(spec.call, "io.print" | "io.printError");
            // App mode routes io output to the AppKit transcript window
            // (plan-04-macos-app.md §5.4) instead of a file descriptor.
            let (frame, instructions, relocations) = if app_mode {
                platform
                    .emit_app_io_write_helper(symbol, stderr, newline, term_state_offset, platform_imports)
                    .ok_or_else(|| {
                        format!(
                            "native target '{}' does not support app-mode io helpers",
                            platform.target()
                        )
                    })??
            } else {
                lower_io_write_helper(symbol, platform_imports, platform, stderr, newline)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.flush" | "io.flushError" => {
            // App-mode transcript writes are synchronous (each io write blocks on
            // the main thread via performSelectorOnMainThread), so output is
            // already visible; flush succeeds immediately (plan §5.4).
            let (frame, instructions, relocations) = if app_mode {
                platform
                    .emit_app_io_flush_helper(symbol)
                    .ok_or_else(|| {
                        format!(
                            "native target '{}' does not support app-mode io helpers",
                            platform.target()
                        )
                    })??
            } else {
                lower_io_flush_helper(
                    symbol,
                    platform_imports,
                    platform,
                    matches!(spec.call, "io.flushError"),
                )?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.pollInput" => {
            let (frame, instructions, relocations) =
                lower_io_poll_input_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.input" | "io.readLine" => {
            // App-mode io.input writes its prompt to the transcript (via io.write)
            // then reads a line (via io.readLine); io.readLine itself is the
            // unchanged console helper, which reads fd 0 — the window input pipe
            // in app mode (plan §5.4). All other read helpers are likewise
            // unchanged and read the pipe.
            let (frame, instructions, relocations) = if app_mode && spec.call == "io.input" {
                platform.emit_app_io_input_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??
            } else {
                lower_io_read_line_helper(
                    symbol,
                    platform_imports,
                    platform,
                    spec.call == "io.input",
                )?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.readChar" => {
            let (frame, instructions, relocations) =
                lower_io_read_char_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.readByte" => {
            let (frame, instructions, relocations) =
                lower_io_read_byte_helper(symbol, platform_imports, platform, app_mode)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
            let fd = match spec.call {
                "io.isInputTerminal" => 0,
                "io.isOutputTerminal" => 1,
                "io.isErrorTerminal" => 2,
                _ => unreachable!(),
            };
            // App mode: the window is the interactive console, so these return
            // TRUE rather than probing a file descriptor (plan §5.4).
            let (frame, instructions, relocations) = if app_mode {
                platform.emit_app_io_is_terminal_helper(symbol).ok_or_else(|| {
                    format!(
                        "native target '{}' does not support app-mode io helpers",
                        platform.target()
                    )
                })??
            } else {
                lower_io_is_terminal_helper(symbol, platform_imports, platform, fd)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.exists" => {
            let (frame, instructions, relocations) =
                lower_fs_exists_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.fileExists" | "fs.directoryExists" => {
            let kind = if spec.call == "fs.fileExists" {
                FS_MODE_REGULAR
            } else {
                FS_MODE_DIRECTORY
            };
            let (frame, instructions, relocations) =
                lower_fs_kind_exists_helper(symbol, platform_imports, platform, kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.currentDirectory" | "fs.tempDirectory" => {
            let (frame, instructions, relocations) = if spec.call == "fs.currentDirectory" {
                lower_fs_current_directory_helper(symbol, platform_imports, platform)?
            } else {
                lower_fs_temp_directory_helper(symbol, platform_imports, platform)?
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.setCurrentDirectory"
        | "fs.deleteFile"
        | "fs.createDirectory"
        | "fs.deleteDirectory" => {
            let operation = match spec.call {
                "fs.setCurrentDirectory" => FsPathOperation::Chdir,
                "fs.deleteFile" => FsPathOperation::Unlink,
                "fs.createDirectory" => FsPathOperation::Mkdir,
                "fs.deleteDirectory" => FsPathOperation::Rmdir,
                _ => unreachable!(),
            };
            let (frame, instructions, relocations) =
                lower_fs_path_operation_helper(symbol, platform_imports, platform, operation)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.createDirectories" => {
            let (frame, instructions, relocations) =
                lower_fs_create_directories_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.listDirectory" => {
            let (frame, instructions, relocations) =
                lower_fs_list_directory_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" => {
            let no_follow = spec.call == "fs.openFileNoFollow";
            let (frame, instructions, relocations) =
                lower_fs_open_helper(symbol, platform_imports, platform, no_follow)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.createTempFile" => {
            let (frame, instructions, relocations) =
                lower_fs_create_temp_file_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.close" => {
            let (frame, instructions, relocations) =
                lower_fs_close_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeAll" => {
            let (frame, instructions, relocations) =
                lower_fs_write_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeAllBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_write_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readText" => {
            let (frame, instructions, relocations) =
                lower_fs_read_text_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_read_bytes_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeText" | "fs.appendText" => {
            let append = spec.call == "fs.appendText";
            let (frame, instructions, relocations) =
                lower_fs_write_text_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeBytes" | "fs.appendBytes" => {
            let append = spec.call == "fs.appendBytes";
            let (frame, instructions, relocations) =
                lower_fs_write_bytes_path_helper(symbol, platform_imports, platform, append)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.writeTextAtomic" | "fs.writeBytesAtomic" => {
            let value_kind = if spec.call == "fs.writeTextAtomic" {
                AtomicWriteValueKind::String
            } else {
                AtomicWriteValueKind::Bytes
            };
            let (frame, instructions, relocations) =
                lower_fs_atomic_write_helper(symbol, platform_imports, platform, value_kind)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readAll" => {
            let (frame, instructions, relocations) =
                lower_fs_read_all_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readAllBytes" => {
            let (frame, instructions, relocations) =
                lower_fs_read_all_bytes_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.readLine" => {
            let (frame, instructions, relocations) =
                lower_fs_read_line_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.eof" => {
            let (frame, instructions, relocations) =
                lower_fs_eof_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.canonicalPath" => {
            let (frame, instructions, relocations) =
                lower_fs_canonical_path_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "fs.isWithin" => {
            let (frame, instructions, relocations) =
                lower_fs_is_within_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        "strings.trim"
        | "strings.trimStart"
        | "strings.trimEnd"
        | "strings.upper"
        | "strings.lower"
        | "strings.caseFold"
        | "strings.normalizeNfc"
        | "strings.graphemes"
        | "strings.startsWith"
        | "strings.endsWith"
        | "strings.contains"
        | "strings.split"
        | "strings.join"
        | "strings.byteLen" => lower_direct_builtin_runtime_helper(symbol, spec, platform_imports),
        "thread.start" | "thread.isRunning" | "thread.waitFor" | "thread.cancel"
        | "thread.drop" | "thread.send" | "thread.poll" | "thread.read" | "thread.receive"
        | "thread.emit" | "thread.transferResource" | "thread.acceptResource"
        | "thread.isCancelled" => {
            let (frame, instructions, relocations) =
                lower_thread_helper(symbol, spec.call, uses_rng, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        call if call.starts_with("net.") => {
            let (frame, instructions, relocations) = match call {
                "net.lookup" => net::lower_net_lookup_helper(symbol, platform_imports, platform)?,
                "net.connectTcp" => {
                    net::lower_net_connect_tcp_helper(symbol, platform_imports, platform)?
                }
                "net.connectTcpAddr" => {
                    net::lower_net_connect_tcp_addr_helper(symbol, platform_imports, platform)?
                }
                "net.listenTcp" => {
                    net::lower_net_listen_tcp_helper(symbol, platform_imports, platform)?
                }
                "net.accept" => net::lower_net_accept_helper(symbol, platform_imports, platform)?,
                "net.poll" => net::lower_net_poll_helper(symbol, platform_imports, platform)?,
                "net.read" => {
                    net::lower_net_read_helper(symbol, platform_imports, platform, false)?
                }
                "net.readText" => {
                    net::lower_net_read_helper(symbol, platform_imports, platform, true)?
                }
                "net.write" => {
                    net::lower_net_write_helper(symbol, platform_imports, platform, false)?
                }
                "net.writeText" => {
                    net::lower_net_write_helper(symbol, platform_imports, platform, true)?
                }
                // A socket/listener handle shares the `File` record layout, so the
                // standard file close helper closes net handles too.
                "net.close" => lower_fs_close_helper(symbol, platform_imports, platform)?,
                "net.localAddress" => {
                    net::lower_net_address_helper(symbol, platform_imports, platform, false)?
                }
                "net.remoteAddress" => {
                    net::lower_net_address_helper(symbol, platform_imports, platform, true)?
                }
                "net.setReadTimeout" => {
                    net::lower_net_set_timeout_helper(symbol, platform_imports, platform, false)?
                }
                "net.setWriteTimeout" => {
                    net::lower_net_set_timeout_helper(symbol, platform_imports, platform, true)?
                }
                "net.bindUdp" => {
                    net::lower_net_bind_udp_helper(symbol, platform_imports, platform)?
                }
                "net.receiveFrom" => {
                    net::lower_net_receive_from_helper(symbol, platform_imports, platform, false)?
                }
                "net.receiveTextFrom" => {
                    net::lower_net_receive_from_helper(symbol, platform_imports, platform, true)?
                }
                "net.sendTo" => {
                    net::lower_net_send_to_helper(symbol, platform_imports, platform, false)?
                }
                "net.sendTextTo" => {
                    net::lower_net_send_to_helper(symbol, platform_imports, platform, true)?
                }
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        call if call.starts_with("tls.") => {
            let (frame, instructions, relocations) = match call {
                "tls.connect" => {
                    tls::lower_tls_connect_helper(symbol, platform_imports, platform)?
                }
                "tls.read" => {
                    tls::lower_tls_read_helper(symbol, platform_imports, platform, false)?
                }
                "tls.readText" => {
                    tls::lower_tls_read_helper(symbol, platform_imports, platform, true)?
                }
                "tls.write" => {
                    tls::lower_tls_write_helper(symbol, platform_imports, platform, false)?
                }
                "tls.writeText" => {
                    tls::lower_tls_write_helper(symbol, platform_imports, platform, true)?
                }
                "tls.close" => tls::lower_tls_close_helper(symbol, platform_imports, platform)?,
                other => {
                    return Err(format!(
                        "native code plan does not emit runtime call '{other}'"
                    ));
                }
            };
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        other => Err(format!(
            "native code plan does not emit runtime call '{other}'"
        )),
    }
}

fn lower_direct_builtin_runtime_helper(
    symbol: &str,
    spec: &runtime::RuntimeHelperSpec,
    platform_imports: &HashMap<String, String>,
) -> Result<CodeFunction, String> {
    let function_symbols = HashMap::new();
    let functions = HashMap::new();
    let package_return_types = HashMap::new();
    let globals = HashMap::new();
    let string_symbols = standard_error_messages()
        .iter()
        .map(|(_, message, symbol)| ((*message).to_string(), (*symbol).to_string()))
        .collect::<HashMap<_, _>>();
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports,
        globals: &globals,
        type_model: TypeModel::empty(),
        string_symbols: &string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
        raw_result_capture: None,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_collections: HashSet::new(),
        owned_list_heads: HashMap::new(),
    };

    let args = spec
        .abi
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let slot = builder.allocate_stack_object(param.name, 8);
            builder.locals.insert(
                param.name.to_string(),
                LocalValue {
                    type_: param.type_.to_string(),
                    stack_offset: slot,
                    constant: None,
                },
            );
            builder.emit(abi::store_u64(
                &abi::argument_register(index)?,
                abi::stack_pointer(),
                slot,
            ));
            Ok(NirValue::Local(param.name.to_string()))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let result = builder.lower_value(&NirValue::Call {
        target: spec.call.to_string(),
        args,
        loc: NirSourceLoc::default(),
    })?;
    if spec.abi.returns != "Nothing" {
        builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    }
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("runtime.{}", spec.call),
        symbol: symbol.to_string(),
        params: spec
            .abi
            .params
            .iter()
            .map(|param| CodeParam {
                name: param.name.to_string(),
                type_: param.type_.to_string(),
                location: param.location.to_string(),
            })
            .collect(),
        returns: spec.abi.returns.to_string(),
        frame,
        stack_slots,
        instructions,
        relocations: builder.relocations,
    })
}

fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(16)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::load_u64(abi::string_length_register(), abi::return_register(), 0),
        abi::add_immediate(abi::string_data_register(), abi::return_register(), 8),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            if stderr { "2" } else { "1" },
        ),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
    ]);
    if append_newline {
        instructions.extend([
            abi::move_immediate(abi::newline_scratch_register(), "Integer", "10"),
            abi::store_u64(abi::newline_scratch_register(), abi::stack_pointer(), 8),
            abi::move_immediate(
                abi::return_register(),
                "Integer",
                if stderr { "2" } else { "1" },
            ),
            abi::add_immediate(abi::string_data_register(), abi::stack_pointer(), 8),
            abi::move_immediate(abi::string_length_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&write_error),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let output_error_symbol = ERR_OUTPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(16),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(16), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

const THREAD_BLOCK_SIZE: usize = 112;
const THREAD_OFFSET_STATE: usize = 0;
const THREAD_OFFSET_CANCELLED: usize = 8;
const THREAD_OFFSET_RESULT_TAG: usize = 16;
const THREAD_OFFSET_RESULT_VALUE: usize = 24;
const THREAD_OFFSET_RESULT_ERROR: usize = 32;
const THREAD_OFFSET_INBOUND_QUEUE: usize = 40;
const THREAD_OFFSET_OUTBOUND_QUEUE: usize = 48;
const THREAD_OFFSET_OS_HANDLE: usize = 56;
const THREAD_OFFSET_ENTRY: usize = 64;
const THREAD_OFFSET_DATA: usize = 72;
const THREAD_OFFSET_ARENA_STATE: usize = 80;
const THREAD_OFFSET_PARENT_ARENA_STATE: usize = 88;
// Origin `ErrorLoc` pointer of a worker's terminal error, captured by the
// trampoline so `thread::waitFor` can recover the worker's source location.
const THREAD_OFFSET_RESULT_SOURCE: usize = 96;
// Resource plane (§7): a dedicated parent→worker queue for `thread::transfer`/
// `thread::accept`, independent of the data-channel inbound/outbound queues so a
// thread can carry both planes at once.
const THREAD_OFFSET_RESOURCE_QUEUE: usize = 104;
const THREAD_STATE_RUNNING: &str = "0";
const THREAD_STATE_COMPLETED: &str = "1";
const THREAD_STATE_CLOSED: &str = "2";

const THREAD_QUEUE_NOT_EMPTY_OFFSET: usize = 64;
const THREAD_QUEUE_NOT_FULL_OFFSET: usize = 128;
const THREAD_QUEUE_CAPACITY_OFFSET: usize = 192;
const THREAD_QUEUE_COUNT_OFFSET: usize = 200;
const THREAD_QUEUE_HEAD_OFFSET: usize = 208;
const THREAD_QUEUE_TAIL_OFFSET: usize = 216;
const THREAD_QUEUE_CLOSED_OFFSET: usize = 224;
const THREAD_QUEUE_VALUES_OFFSET: usize = 232;
const THREAD_QUEUE_BLOCK_SIZE: usize = 240;

fn thread_symbol(platform: &dyn CodegenPlatform, name: &str) -> String {
    if platform.target() == "macos-aarch64" {
        format!("_{name}")
    } else {
        name.to_string()
    }
}

fn emit_thread_external_call(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    name: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let symbol = thread_symbol(platform, name);
    instructions.push(abi::branch_link(&symbol));
    relocations.push(external_branch(from, &symbol, platform_imports)?);
    Ok(())
}

fn emit_thread_queue_alloc(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    limit_stack_offset: usize,
    cb_stack_offset: usize,
    queue_stack_offset: usize,
    cb_queue_offset: usize,
    done_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let alloc_queue_ok = format!("{symbol}_queue_{cb_queue_offset}_alloc_ok");
    let alloc_values_ok = format!("{symbol}_queue_{cb_queue_offset}_values_ok");
    let init_error = format!("{symbol}_queue_{cb_queue_offset}_init_error");
    let init_done = format!("{symbol}_queue_{cb_queue_offset}_init_done");

    instructions.extend([
        abi::move_immediate("x0", "Integer", &THREAD_QUEUE_BLOCK_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_queue_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_queue_ok),
        abi::store_u64("x1", abi::stack_pointer(), queue_stack_offset),
        abi::load_u64("x9", abi::stack_pointer(), cb_stack_offset),
        abi::store_u64("x1", "x9", cb_queue_offset),
        abi::load_u64("x10", abi::stack_pointer(), limit_stack_offset),
        abi::store_u64("x10", "x1", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_HEAD_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_TAIL_OFFSET),
        abi::store_u64("x31", "x1", THREAD_QUEUE_CLOSED_OFFSET),
        abi::move_immediate("x11", "Integer", "8"),
        abi::multiply_registers("x0", "x10", "x11"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_values_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALLOCATION_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done_label),
        abi::label(&alloc_values_ok),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::store_u64("x1", "x9", THREAD_QUEUE_VALUES_OFFSET),
        abi::move_register("x0", "x9"),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::load_u64("x9", abi::stack_pointer(), queue_stack_offset),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_init",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&init_error),
        abi::branch(&init_done),
        abi::label(&init_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INTERRUPTED_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done_label));
    instructions.push(abi::label(&init_done));
    Ok(())
}

fn lower_thread_helper(
    symbol: &str,
    call: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    match call {
        "thread.start" => lower_thread_start_helper(symbol, uses_rng, platform_imports, platform),
        "thread.isRunning" => simple_thread_handle_helper(
            symbol,
            ThreadSimpleOp::IsRunning,
            platform_imports,
            platform,
        ),
        "thread.waitFor" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::WaitFor, platform_imports, platform)
        }
        "thread.cancel" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Cancel, platform_imports, platform)
        }
        "thread.drop" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Drop, platform_imports, platform)
        }
        "thread.send" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.poll" => {
            simple_thread_handle_helper(symbol, ThreadSimpleOp::Poll, platform_imports, platform)
        }
        "thread.read" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        "thread.receive" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_INBOUND_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.emit" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_OUTBOUND_QUEUE,
            false,
            platform_imports,
            platform,
        ),
        // Resource plane: transfer/accept mirror send/receive on the dedicated
        // resource queue (parent → worker).
        "thread.transferResource" => thread_queue_write_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.acceptResource" => thread_queue_read_helper(
            symbol,
            THREAD_OFFSET_RESOURCE_QUEUE,
            true,
            platform_imports,
            platform,
        ),
        "thread.isCancelled" => Ok(thread_is_cancelled_helper()),
        _ => Err(format!("native thread helper does not implement {call}")),
    }
}

fn lower_thread_start_helper(
    symbol: &str,
    uses_rng: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const ENTRY_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const IN_LIMIT_OFFSET: usize = 24;
    const OUT_LIMIT_OFFSET: usize = 32;
    const CB_OFFSET: usize = 40;
    const QUEUE_OFFSET: usize = 48;

    let invalid_limit = format!("{symbol}_invalid_limit");
    let alloc_block_ok = format!("{symbol}_alloc_block_ok");
    let alloc_worker_arena_ok = format!("{symbol}_alloc_worker_arena_ok");
    let spawn_error = format!("{symbol}_spawn_error");
    let parent_done = format!("{symbol}_parent_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();

    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x0", abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x2", abi::stack_pointer(), IN_LIMIT_OFFSET),
        abi::store_u64("x3", abi::stack_pointer(), OUT_LIMIT_OFFSET),
        abi::compare_immediate("x2", "1"),
        abi::branch_lt(&invalid_limit),
        abi::compare_immediate("x3", "1"),
        abi::branch_lt(&invalid_limit),
        abi::move_immediate("x0", "Integer", &THREAD_BLOCK_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_block_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_block_ok),
        abi::store_u64("x1", abi::stack_pointer(), CB_OFFSET),
        abi::move_register("x9", "x1"),
        abi::store_u64("x31", "x9", THREAD_OFFSET_STATE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_CANCELLED),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_TAG),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_VALUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_ERROR),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESULT_SOURCE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_INBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_RESOURCE_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_OS_HANDLE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_PARENT_ARENA_STATE),
        abi::load_u64("x10", abi::stack_pointer(), ENTRY_OFFSET),
        abi::store_u64("x10", "x9", THREAD_OFFSET_ENTRY),
        abi::load_u64("x10", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x10", "x9", THREAD_OFFSET_DATA),
        abi::store_u64(ARENA_STATE_REGISTER, "x9", THREAD_OFFSET_PARENT_ARENA_STATE),
        abi::move_immediate("x0", "Integer", &ARENA_STATE_SIZE.to_string()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_eq(&alloc_worker_arena_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&alloc_worker_arena_ok),
        abi::store_u64("x31", "x1", 0),
        abi::store_u64("x31", "x1", 8),
        abi::store_u64("x31", "x1", 16),
        abi::store_u64("x31", "x1", 24),
        abi::store_u64("x31", "x1", 32),
        abi::store_u64("x31", "x1", 40),
        abi::store_u64("x31", "x1", 48),
        abi::store_u64("x31", "x1", 56),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_COUNT_OFFSET),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_CODE_OFFSET),
        abi::store_u64("x31", "x1", ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::store_u64("x1", "x9", THREAD_OFFSET_ARENA_STATE),
    ]);

    if uses_rng {
        // Give the new thread its own PCG64 stream by drawing a 64-bit seed from
        // the spawning thread's generator (runs in the parent, so `x19` is the
        // parent arena and the draw is race-free). Reload the child arena from
        // the control block afterwards because the draw clobbers x0-x18.
        instructions.push(abi::branch_link(RNG_NEXT_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_NEXT_SYMBOL));
        instructions.extend([
            abi::move_register("x1", abi::return_register()),
            abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
            abi::load_u64(abi::return_register(), "x9", THREAD_OFFSET_ARENA_STATE),
        ]);
        instructions.push(abi::branch_link(RNG_SEED_SYMBOL));
        relocations.push(internal_branch(symbol, RNG_SEED_SYMBOL));
    }

    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_INBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;
    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        OUT_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_OUTBOUND_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;
    // Resource plane queue (§7): bounded like the inbound data queue.
    emit_thread_queue_alloc(
        symbol,
        platform_imports,
        platform,
        IN_LIMIT_OFFSET,
        CB_OFFSET,
        QUEUE_OFFSET,
        THREAD_OFFSET_RESOURCE_QUEUE,
        &parent_done,
        &mut instructions,
        &mut relocations,
    )?;

    let pthread_create_symbol = if platform.target() == "macos-aarch64" {
        "_pthread_create"
    } else {
        "pthread_create"
    };
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_OFFSET_OS_HANDLE),
        abi::move_immediate("x1", "Integer", "0"),
    ]);
    instructions.push(abi::load_page_address("x2", THREAD_TRAMPOLINE_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: "page21".to_string(),
        binding: "data".to_string(),
        library: None,
    });
    instructions.push(abi::add_page_offset("x2", "x2", THREAD_TRAMPOLINE_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        kind: "pageoff12".to_string(),
        binding: "data".to_string(),
        library: None,
    });
    instructions.extend([
        abi::move_register("x3", "x9"),
        abi::branch_link(pthread_create_symbol),
    ]);
    relocations.push(external_branch(
        symbol,
        pthread_create_symbol,
        platform_imports,
    )?);
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&spawn_error),
        abi::load_u64("x9", abi::stack_pointer(), CB_OFFSET),
        abi::move_register(RESULT_VALUE_REGISTER, "x9"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&parent_done),
    ]);

    instructions.extend([
        abi::label(&invalid_limit),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&parent_done),
        abi::label(&spawn_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&parent_done));
    instructions.extend([
        abi::label(&parent_done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_thread_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const ARENA_OFFSET: usize = 8;
    const X20_OFFSET: usize = 16;
    const CLOSURE_OFFSET: usize = 24;
    const CB_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const VALUE_OFFSET: usize = 48;
    const ERROR_OFFSET: usize = 56;
    const SOURCE_OFFSET: usize = 64;
    let result_closed = format!("{THREAD_TRAMPOLINE_SYMBOL}_result_closed");

    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::store_u64("x20", abi::stack_pointer(), X20_OFFSET),
        abi::store_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::move_register("x20", "x0"),
        abi::store_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64(ARENA_STATE_REGISTER, "x20", THREAD_OFFSET_ARENA_STATE),
        abi::load_u64("x9", "x20", THREAD_OFFSET_ENTRY),
        abi::load_u64(CLOSURE_ENV_REGISTER, "x9", CLOSURE_OFFSET_ENV),
        abi::load_u64("x9", "x9", CLOSURE_OFFSET_CODE),
        abi::load_u64("x1", "x20", THREAD_OFFSET_DATA),
        abi::move_register("x0", "x20"),
        abi::branch_link_register("x9"),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            SOURCE_OFFSET,
        ),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::move_register("x0", "x9"),
    ];
    let mut relocations = Vec::new();
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::move_immediate("x10", "Integer", "1"),
        abi::store_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_INBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x0", "x20", THREAD_OFFSET_INBOUND_QUEUE),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    // Close the resource-plane queue on worker exit, mirroring the inbound data
    // queue: wake any parent blocked in `thread::transfer`.
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_RESOURCE_QUEUE),
        abi::move_register("x0", "x9"),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_RESOURCE_QUEUE),
        abi::move_immediate("x10", "Integer", "1"),
        abi::store_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_RESOURCE_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x0", "x20", THREAD_OFFSET_RESOURCE_QUEUE),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::move_register("x0", "x9"),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_STATE),
        abi::compare_immediate("x9", THREAD_STATE_CLOSED),
        abi::branch_eq(&result_closed),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_TAG),
        abi::load_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_VALUE),
        abi::load_u64("x9", abi::stack_pointer(), ERROR_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_ERROR),
        abi::load_u64("x9", abi::stack_pointer(), SOURCE_OFFSET),
        abi::store_u64("x9", "x20", THREAD_OFFSET_RESULT_SOURCE),
        abi::move_immediate("x10", "Integer", THREAD_STATE_COMPLETED),
        abi::store_u64("x10", "x20", THREAD_OFFSET_STATE),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x9", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_cond_broadcast",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&result_closed),
        abi::load_u64("x20", abi::stack_pointer(), CB_OFFSET),
        abi::load_u64("x0", "x20", THREAD_OFFSET_OUTBOUND_QUEUE),
    ]);
    emit_thread_external_call(
        THREAD_TRAMPOLINE_SYMBOL,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate("x0", "Integer", "0"),
        abi::load_u64(ARENA_STATE_REGISTER, abi::stack_pointer(), ARENA_OFFSET),
        abi::load_u64(CLOSURE_ENV_REGISTER, abi::stack_pointer(), CLOSURE_OFFSET),
        abi::load_u64("x20", abi::stack_pointer(), X20_OFFSET),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.thread.trampoline".to_string(),
        symbol: THREAD_TRAMPOLINE_SYMBOL.to_string(),
        params: vec![CodeParam {
            name: "controlBlock".to_string(),
            type_: "ThreadControlBlock".to_string(),
            location: "x0".to_string(),
        }],
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string(), "x20".to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

enum ThreadSimpleOp {
    IsRunning,
    WaitFor,
    Cancel,
    Drop,
    Poll,
}

fn emit_thread_deadline(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    timeout_stack_offset: usize,
    timespec_stack_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let done = format!("{symbol}_deadline_done_{timespec_stack_offset}");
    let nsec_ok = format!("{symbol}_deadline_nsec_ok_{timespec_stack_offset}");
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), timeout_stack_offset),
        abi::compare_immediate("x9", "0"),
        abi::branch_le(&done),
        abi::move_immediate("x0", "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), timespec_stack_offset),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "clock_gettime",
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), timeout_stack_offset),
        abi::move_immediate("x10", "Integer", "1000"),
        abi::signed_divide_registers("x11", "x9", "x10"),
        abi::multiply_subtract_registers("x12", "x11", "x10", "x9"),
        abi::move_immediate("x13", "Integer", "1000000"),
        abi::multiply_registers("x12", "x12", "x13"),
        abi::load_u64("x14", abi::stack_pointer(), timespec_stack_offset),
        abi::add_registers("x14", "x14", "x11"),
        abi::load_u64("x15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::add_registers("x15", "x15", "x12"),
        abi::move_immediate("x13", "Integer", "1000000000"),
        abi::compare_registers("x15", "x13"),
        abi::branch_lt(&nsec_ok),
        abi::subtract_registers("x15", "x15", "x13"),
        abi::add_immediate("x14", "x14", 1),
        abi::label(&nsec_ok),
        abi::store_u64("x14", abi::stack_pointer(), timespec_stack_offset),
        abi::store_u64("x15", abi::stack_pointer(), timespec_stack_offset + 8),
        abi::label(&done),
    ]);
    Ok(())
}

fn simple_thread_handle_helper(
    symbol: &str,
    op: ThreadSimpleOp,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const HANDLE_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TAG_OFFSET: usize = 24;
    const ERROR_OFFSET: usize = 32;
    // WaitFor only: origin ErrorLoc of a propagated worker error (0 otherwise).
    const SOURCE_OFFSET: usize = 40;

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x0", abi::stack_pointer(), HANDLE_OFFSET),
    ]);
    match op {
        ThreadSimpleOp::IsRunning => {
            let running = format!("{symbol}_running");
            let closed = format!("{symbol}_closed");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("x9", "x0", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::store_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("x9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("x9", THREAD_STATE_RUNNING),
                abi::branch_eq(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&running),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([abi::label(&done)]);
        }
        ThreadSimpleOp::WaitFor => {
            let loop_label = format!("{symbol}_wait_loop");
            let closed = format!("{symbol}_closed");
            let result_ready = format!("{symbol}_result_ready");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("x9", "x0", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::label(&loop_label),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::compare_immediate("x9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::compare_immediate("x9", THREAD_STATE_COMPLETED),
                abi::branch_eq(&result_ready),
                abi::load_u64("x9", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register("x1", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_wait",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::branch(&loop_label),
                abi::label(&result_ready),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    "x8",
                    THREAD_OFFSET_RESULT_ERROR,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, "x8", THREAD_OFFSET_RESULT_VALUE),
                abi::load_u64(RESULT_TAG_REGISTER, "x8", THREAD_OFFSET_RESULT_TAG),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    "x8",
                    THREAD_OFFSET_RESULT_SOURCE,
                ),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
                abi::move_immediate("x9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::load_u64("x10", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("x9", "x10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_COUNT_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_detach",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::branch(&done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                // waitFor's own error (resource closed): no worker origin.
                abi::store_u64("x31", abi::stack_pointer(), SOURCE_OFFSET),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::label(&done),
                abi::load_u64(
                    RESULT_ERROR_SOURCE_REGISTER,
                    abi::stack_pointer(),
                    SOURCE_OFFSET,
                ),
            ]);
        }
        ThreadSimpleOp::Cancel => {
            let closed = format!("{symbol}_closed");
            let closed_unlocked = format!("{symbol}_closed_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            instructions.extend([
                abi::load_u64("x9", "x0", THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::compare_immediate("x9", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::move_immediate("x9", "Integer", "1"),
                abi::store_u64("x9", "x8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("x10", "x8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("x9", "x10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x10", "x8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::label(&inbound_unlocked),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("x9", "Integer", "1"),
                abi::load_u64("x10", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("x9", "x10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x10", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&closed_unlocked),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&closed_unlocked),
            ]);
        }
        ThreadSimpleOp::Drop => {
            let already_closed = format!("{symbol}_already_closed");
            let outbound_unlocked = format!("{symbol}_outbound_unlocked");
            let inbound_unlocked = format!("{symbol}_inbound_unlocked");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::load_u64("x9", "x0", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::store_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("x9", THREAD_STATE_CLOSED),
                abi::branch_eq(&already_closed),
                abi::move_immediate("x9", "Integer", THREAD_STATE_CLOSED),
                abi::store_u64("x9", "x8", THREAD_OFFSET_STATE),
                abi::load_u64("x10", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::store_u64("x9", "x10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x10", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::label(&already_closed));
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x9", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("x9", THREAD_STATE_CLOSED),
                abi::branch_eq(&done),
                abi::label(&outbound_unlocked),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::move_immediate("x9", "Integer", "1"),
                abi::store_u64("x9", "x8", THREAD_OFFSET_CANCELLED),
                abi::load_u64("x10", "x8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::store_u64("x9", "x10", THREAD_QUEUE_CLOSED_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_COUNT_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_HEAD_OFFSET),
                abi::store_u64("x31", "x10", THREAD_QUEUE_TAIL_OFFSET),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_EMPTY_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x10", "x8", THREAD_OFFSET_INBOUND_QUEUE),
                abi::add_immediate("x0", "x10", THREAD_QUEUE_NOT_FULL_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_broadcast",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_INBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::label(&inbound_unlocked),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OS_HANDLE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_detach",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::label(&done),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            ]);
        }
        ThreadSimpleOp::Poll => {
            let ready = format!("{symbol}_ready");
            let closed = format!("{symbol}_closed");
            let invalid = format!("{symbol}_invalid_timeout");
            let wait_loop = format!("{symbol}_wait_loop");
            let wait_timed = format!("{symbol}_wait_timed");
            let not_ready = format!("{symbol}_not_ready");
            let locked_done = format!("{symbol}_locked_done");
            let done = format!("{symbol}_done");
            instructions.extend([
                abi::compare_immediate("x1", "0"),
                abi::branch_lt(&invalid),
                abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
            ]);
            emit_thread_deadline(
                symbol,
                platform_imports,
                platform,
                VALUE_OFFSET,
                ERROR_OFFSET,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::move_register("x0", "x9"),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_lock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::label(&wait_loop),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x9", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
                abi::load_u64("x10", "x8", THREAD_OFFSET_STATE),
                abi::compare_immediate("x10", THREAD_STATE_CLOSED),
                abi::branch_eq(&closed),
                abi::load_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
                abi::compare_immediate("x10", "0"),
                abi::branch_gt(&ready),
                abi::load_u64("x10", "x8", THREAD_OFFSET_STATE),
                abi::compare_immediate("x10", THREAD_STATE_COMPLETED),
                abi::branch_eq(&not_ready),
                abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
                abi::compare_immediate("x10", "0"),
                abi::branch_gt(&wait_timed),
                abi::branch(&not_ready),
                abi::label(&wait_timed),
                abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
                abi::move_register("x1", "x9"),
                abi::add_immediate("x2", abi::stack_pointer(), ERROR_OFFSET),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_cond_timedwait",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::compare_immediate("x0", "0"),
                abi::branch_ne(&not_ready),
                abi::branch(&wait_loop),
                abi::label(&ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&not_ready),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::branch(&locked_done),
                abi::label(&closed),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_RESOURCE_CLOSED_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::branch(&locked_done),
                abi::label(&invalid),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                symbol,
                ERR_INVALID_ARGUMENT_SYMBOL,
                &mut instructions,
                &mut relocations,
            );
            instructions.extend([
                abi::branch(&done),
                abi::label(&locked_done),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
                abi::load_u64("x0", "x8", THREAD_OFFSET_OUTBOUND_QUEUE),
            ]);
            emit_thread_external_call(
                symbol,
                platform_imports,
                platform,
                "pthread_mutex_unlock",
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
                abi::load_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::label(&done),
            ]);
        }
    }
    instructions.extend([
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn thread_queue_write_helper(
    symbol: &str,
    queue_offset: usize,
    parent_send: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const HANDLE_OFFSET: usize = 8;
    const DATA_OFFSET: usize = 16;
    const TIMEOUT_OFFSET: usize = 24;
    const QUEUE_OFFSET: usize = 32;
    const TIMESPEC_OFFSET: usize = 40;

    let invalid = format!("{symbol}_invalid");
    let closed = format!("{symbol}_closed");
    let interrupted = format!("{symbol}_interrupted");
    let timeout = format!("{symbol}_timeout");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let enqueue = format!("{symbol}_enqueue");
    let tail_wrap = format!("{symbol}_tail_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x0", abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x2", "0"),
        abi::branch_lt(&invalid),
    ]);
    if !parent_send {
        // Re-establish the current-thread register `x20` from the worker's own
        // control block (`x0`) rather than asserting equality; see the matching
        // note in `thread_queue_read_helper`.
        instructions.push(abi::move_register("x20", "x0"));
    }
    emit_thread_deadline(
        symbol,
        platform_imports,
        platform,
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("x9", "x8", queue_offset),
        abi::store_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register("x0", "x9"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&wait_loop));
    if parent_send {
        instructions.extend([
            abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("x9", "x8", THREAD_OFFSET_STATE),
            abi::compare_immediate("x9", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("x9", THREAD_STATE_COMPLETED),
            abi::branch_eq(&interrupted),
            abi::load_u64("x9", "x8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("x9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    } else {
        instructions.extend([
            abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("x9", "x8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("x9", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne(&interrupted),
        abi::load_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::load_u64("x11", "x9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&enqueue),
        abi::load_u64("x12", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&timeout),
        abi::label(&wait_timed),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
        abi::move_register("x1", "x9"),
        abi::add_immediate("x2", abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_timedwait",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&enqueue),
        abi::load_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("x11", "x9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("x12", "x10", 3),
        abi::add_registers("x11", "x11", "x12"),
        abi::load_u64("x12", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x12", "x11", 0),
        abi::add_immediate("x10", "x10", 1),
        abi::load_u64("x11", "x9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&tail_wrap),
        abi::move_immediate("x10", "Integer", "0"),
        abi::label(&tail_wrap),
        abi::store_u64("x10", "x9", THREAD_QUEUE_TAIL_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate("x10", "x10", 1),
        abi::store_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_signal",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&interrupted),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&timeout),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_TIMEOUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::load_u64("x0", abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), DATA_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            TIMESPEC_OFFSET,
        ),
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn thread_queue_read_helper(
    symbol: &str,
    queue_offset: usize,
    worker_only: bool,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const HANDLE_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;
    const QUEUE_OFFSET: usize = 24;
    const VALUE_OFFSET: usize = 32;
    const TAG_OFFSET: usize = 40;
    const ERROR_OFFSET: usize = 48;
    const TIMESPEC_OFFSET: usize = 56;

    let invalid = format!("{symbol}_invalid");
    let found = format!("{symbol}_found");
    let wait_loop = format!("{symbol}_wait_loop");
    let wait_timed = format!("{symbol}_wait_timed");
    let wait_indefinite = format!("{symbol}_wait_indefinite");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let not_found = format!("{symbol}_not_found");
    let interrupted = format!("{symbol}_interrupted");
    let closed = format!("{symbol}_closed");
    let timeout = format!("{symbol}_timeout");
    let head_wrap = format!("{symbol}_head_wrap");
    let unlock = format!("{symbol}_unlock");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x0", abi::stack_pointer(), HANDLE_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), TIMEOUT_OFFSET),
    ]);
    if worker_only {
        instructions.extend([
            // The caller's `x0` is this worker's own control block (the handle is
            // unforgeable in type-correct code). Re-establish the current-thread
            // register `x20` from it rather than asserting equality: arbitrary
            // generated code between worker ops (e.g. arena allocation) may clobber
            // `x20`, so we restore the invariant here instead of failing on it.
            abi::move_register("x20", "x0"),
            abi::compare_immediate("x1", "0"),
            abi::branch_ge(&timeout_ok),
            abi::add_immediate("x9", "x1", 1),
            abi::compare_immediate("x9", "0"),
            abi::branch_ne(&invalid),
            abi::label(&timeout_ok),
        ]);
    } else {
        instructions.extend([abi::compare_immediate("x1", "0"), abi::branch_lt(&invalid)]);
    }
    emit_thread_deadline(
        symbol,
        platform_imports,
        platform,
        TIMEOUT_OFFSET,
        TIMESPEC_OFFSET,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
        abi::load_u64("x9", "x8", queue_offset),
        abi::store_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::move_register("x0", "x9"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_lock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&wait_loop),
        abi::load_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_gt(&found),
    ]);
    if worker_only {
        instructions.extend([
            abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("x10", "x8", THREAD_OFFSET_CANCELLED),
            abi::compare_immediate("x10", "0"),
            abi::branch_ne(&interrupted),
        ]);
    }
    instructions.extend([
        abi::load_u64("x10", "x9", THREAD_QUEUE_CLOSED_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne(&not_found),
    ]);
    if !worker_only {
        instructions.extend([
            abi::load_u64("x8", abi::stack_pointer(), HANDLE_OFFSET),
            abi::load_u64("x10", "x8", THREAD_OFFSET_STATE),
            abi::compare_immediate("x10", THREAD_STATE_CLOSED),
            abi::branch_eq(&closed),
            abi::compare_immediate("x10", THREAD_STATE_COMPLETED),
            abi::branch_eq(&not_found),
        ]);
    }
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&not_found),
        abi::branch_lt(&wait_indefinite),
        abi::label(&wait_timed),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register("x1", "x9"),
        abi::add_immediate("x2", abi::stack_pointer(), TIMESPEC_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_timedwait",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(&timeout),
        abi::branch(&wait_loop),
        abi::label(&wait_indefinite),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_EMPTY_OFFSET),
        abi::move_register("x1", "x9"),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_wait",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::branch(&wait_loop),
        abi::label(&found),
        abi::load_u64("x9", abi::stack_pointer(), QUEUE_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("x11", "x9", THREAD_QUEUE_VALUES_OFFSET),
        abi::shift_left_immediate("x12", "x10", 3),
        abi::add_registers("x11", "x11", "x12"),
        abi::load_u64(RESULT_VALUE_REGISTER, "x11", 0),
        abi::add_immediate("x10", "x10", 1),
        abi::load_u64("x11", "x9", THREAD_QUEUE_CAPACITY_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&head_wrap),
        abi::move_immediate("x10", "Integer", "0"),
        abi::label(&head_wrap),
        abi::store_u64("x10", "x9", THREAD_QUEUE_HEAD_OFFSET),
        abi::load_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::subtract_immediate("x10", "x10", 1),
        abi::store_u64("x10", "x9", THREAD_QUEUE_COUNT_OFFSET),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::add_immediate("x0", "x9", THREAD_QUEUE_NOT_FULL_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_cond_signal",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&unlock),
        abi::label(&not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&interrupted),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INTERRUPTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INTERRUPTED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&timeout),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_TIMEOUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&unlock),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&unlock),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::load_u64("x0", abi::stack_pointer(), QUEUE_OFFSET),
    ]);
    emit_thread_external_call(
        symbol,
        platform_imports,
        platform,
        "pthread_mutex_unlock",
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            ERROR_OFFSET,
        ),
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn thread_is_cancelled_helper() -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let cancelled = "_mfb_rt_thread_is_cancelled_true";
    let done = "_mfb_rt_thread_is_cancelled_done";
    let instructions = vec![
        abi::label("entry"),
        abi::load_u64("x9", "x20", THREAD_OFFSET_CANCELLED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(done),
        abi::label(cancelled),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(done),
        abi::return_(),
    ];
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        Vec::new(),
    )
}

fn internal_branch(from: &str, to: &str) -> CodeRelocation {
    CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }
}

fn external_branch(
    from: &str,
    to: &str,
    platform_imports: &HashMap<String, String>,
) -> Result<CodeRelocation, String> {
    let library = platform_imports
        .get(to)
        .ok_or_else(|| format!("thread runtime helper requires {to} import"))?
        .clone();
    Ok(CodeRelocation {
        from: from.to_string(),
        to: to.to_string(),
        kind: "branch26".to_string(),
        binding: "external".to_string(),
        library: Some(library),
    })
}

fn lower_io_flush_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    const LR_OFFSET: usize = 0;
    const ERRNO_EINVAL: &str = "22";
    const ERRNO_ENOTSUP_DARWIN: &str = "45";
    const ERRNO_EOPNOTSUPP_LINUX: &str = "95";

    let sync_error = format!("{symbol}_sync_error");
    let ok = format!("{symbol}_ok");
    let output_error = format!("{symbol}_output_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        if stderr { "2" } else { "1" },
    ));
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&sync_error),
        abi::label(&ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&sync_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", ERRNO_EINVAL),
        abi::branch_eq(&ok),
        abi::compare_immediate("x9", ERRNO_ENOTSUP_DARWIN),
        abi::branch_eq(&ok),
        abi::compare_immediate("x9", ERRNO_EOPNOTSUPP_LINUX),
        abi::branch_eq(&ok),
        abi::label(&output_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn lower_io_poll_input_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 32;

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate("x9", "Integer", POLLIN_PACKED_FD0),
        abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
    ]);

    instructions.push(abi::load_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET));

    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    platform.emit_poll_input(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;

    let poll_error = format!("{symbol}_poll_error");
    let poll_ready = format!("{symbol}_poll_ready");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_error),
        abi::branch_gt(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let input_error_symbol = ERR_INPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn termios_storage_size(platform: &dyn CodegenPlatform) -> usize {
    platform.termios_size().next_multiple_of(8)
}

struct TerminalModeSlots {
    active: usize,
    saved_tag: usize,
    saved_value: usize,
    saved_message: usize,
    original: usize,
    modified: usize,
}

fn emit_configure_stdin_terminal(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    slots: &TerminalModeSlots,
    disable_echo: bool,
    disable_canonical: bool,
    error_label: &str,
) -> Result<(), String> {
    let skip = format!("{symbol}_terminal_mode_skip");
    instructions.extend([
        abi::store_u64("x31", abi::stack_pointer(), slots.active),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
    ]);
    platform.emit_libc_call(
        "isatty",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&skip),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), slots.original),
    ]);
    platform.emit_libc_call(
        "tcgetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::move_immediate("x9", "Integer", "1"),
        abi::store_u64("x9", abi::stack_pointer(), slots.active),
    ]);

    for offset in (0..termios_storage_size(platform)).step_by(8) {
        instructions.extend([
            abi::load_u64("x9", abi::stack_pointer(), slots.original + offset),
            abi::store_u64("x9", abi::stack_pointer(), slots.modified + offset),
        ]);
    }

    let mut clear_flags = 0;
    if disable_echo {
        clear_flags |= platform.termios_echo_flag();
    }
    if disable_canonical {
        clear_flags |= platform.termios_icanon_flag();
    }
    if clear_flags != 0 {
        let lflag_offset = slots.modified + platform.termios_lflag_offset();
        if platform.termios_lflag_width() == 4 {
            instructions.push(abi::load_u32("x9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::load_u64("x9", abi::stack_pointer(), lflag_offset));
        }
        instructions.extend([
            abi::move_immediate("x10", "Integer", &clear_flags.to_string()),
            abi::bitwise_not("x10", "x10"),
            abi::and_registers("x9", "x9", "x10"),
        ]);
        if platform.termios_lflag_width() == 4 {
            instructions.push(abi::store_u32("x9", abi::stack_pointer(), lflag_offset));
        } else {
            instructions.push(abi::store_u64("x9", abi::stack_pointer(), lflag_offset));
        }
    }

    if disable_canonical {
        let cc_offset = slots.modified + platform.termios_cc_offset();
        instructions.extend([
            abi::move_immediate("x9", "Integer", "1"),
            abi::store_u8(
                "x9",
                abi::stack_pointer(),
                cc_offset + platform.termios_vmin_index(),
            ),
            abi::store_u8(
                "x31",
                abi::stack_pointer(),
                cc_offset + platform.termios_vtime_index(),
            ),
        ]);
    }

    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), slots.modified),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(error_label),
        abi::label(&skip),
    ]);
    Ok(())
}

fn emit_restore_stdin_terminal(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    slots: &TerminalModeSlots,
) -> Result<(), String> {
    let restored = format!("{symbol}_terminal_mode_restored");
    let restore_failed = format!("{symbol}_terminal_mode_restore_failed");
    instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), slots.saved_tag),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), slots.saved_value),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.saved_message,
        ),
        abi::load_u64("x9", abi::stack_pointer(), slots.active),
        abi::compare_immediate("x9", "1"),
        abi::branch_ne(&restored),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), slots.original),
    ]);
    platform.emit_libc_call(
        "tcsetattr",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&restore_failed),
        abi::label(&restored),
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), slots.saved_tag),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), slots.saved_value),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.saved_message,
        ),
        abi::branch(&format!("{symbol}_terminal_mode_restore_done")),
        abi::label(&restore_failed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        instructions,
        relocations,
    );
    instructions.push(abi::label(&format!(
        "{symbol}_terminal_mode_restore_done"
    )));
    Ok(())
}

fn lower_io_read_byte_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 208;
    const LR_OFFSET: usize = 0;
    const BYTE_OFFSET: usize = 8;
    let terminal_slots = TerminalModeSlots {
        active: 16,
        saved_tag: 24,
        saved_value: 32,
        saved_message: 40,
        original: 48,
        modified: 120,
    };
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    emit_configure_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
        true,
        true,
        &input_error,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTE_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&eof),
        abi::load_u8(RESULT_VALUE_REGISTER, abi::stack_pointer(), BYTE_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
    )?;
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn lower_io_is_terminal_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    fd: u8,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    const LR_OFFSET: usize = 0;
    let yes = format!("{symbol}_yes");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        &fd.to_string(),
    ));
    platform.emit_is_terminal(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&yes),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
    ]);
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn lower_io_read_char_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    app_mode: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 224;
    const LR_OFFSET: usize = 0;
    const BYTES_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const RESULT_OFFSET: usize = 24;
    let terminal_slots = TerminalModeSlots {
        active: 32,
        saved_tag: 40,
        saved_value: 48,
        saved_message: 56,
        original: 64,
        modified: 136,
    };
    let read_second = format!("{symbol}_read_second");
    let read_third = format!("{symbol}_read_third");
    let read_fourth = format!("{symbol}_read_fourth");
    let got_len = format!("{symbol}_got_len");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let eof = format!("{symbol}_eof");
    let input_error = format!("{symbol}_input_error");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    if app_mode {
        platform
            .emit_app_raw_input_mode(symbol, &mut instructions, &mut relocations)
            .ok_or_else(|| {
                format!(
                    "native target '{}' does not support app-mode raw input",
                    platform.target()
                )
            })??;
    }
    emit_configure_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
        true,
        true,
        &input_error,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&eof),
        abi::load_u8("x10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("x10", "127"),
        abi::branch_hi(&read_second),
        abi::move_immediate("x11", "Integer", "1"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_second),
        abi::compare_immediate("x10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "223"),
        abi::branch_hi(&read_third),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "2"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_third),
        abi::compare_immediate("x10", "239"),
        abi::branch_hi(&read_fourth),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "224"),
        abi::branch_ne(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("x11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_not_e0")),
        abi::compare_immediate("x10", "237"),
        abi::branch_ne(&format!("{symbol}_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_three_second_ok")),
        abi::label(&format!("{symbol}_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_three_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "3"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::branch(&got_len),
        abi::label(&read_fourth),
        abi::compare_immediate("x10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "244"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "240"),
        abi::branch_ne(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("x11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_not_f0")),
        abi::compare_immediate("x10", "244"),
        abi::branch_ne(&format!("{symbol}_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_four_second_ok")),
        abi::label(&format!("{symbol}_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_four_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "4"),
        abi::store_u64("x11", abi::stack_pointer(), LEN_OFFSET),
        abi::label(&got_len),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::add_immediate("x12", abi::stack_pointer(), BYTES_OFFSET),
        abi::label(&copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    emit_restore_stdin_terminal(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &terminal_slots,
    )?;
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn lower_io_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    with_prompt: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 256;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const CAPACITY_OFFSET: usize = 16;
    const LENGTH_OFFSET: usize = 24;
    const SEQ_LEN_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const BYTES_OFFSET: usize = 48;
    let terminal_slots = TerminalModeSlots {
        active: 56,
        saved_tag: 64,
        saved_value: 72,
        saved_message: 80,
        original: 96,
        modified: 168,
    };
    let prompt_ok = format!("{symbol}_prompt_ok");
    let prompt_flush = format!("{symbol}_prompt_flush");
    let prompt_flush_error = format!("{symbol}_prompt_flush_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let have_sequence = format!("{symbol}_have_sequence");
    let grow = format!("{symbol}_grow");
    let grow_ok = format!("{symbol}_grow_ok");
    let grow_copy_loop = format!("{symbol}_grow_copy_loop");
    let grow_copy_done = format!("{symbol}_grow_copy_done");
    let append_loop = format!("{symbol}_append_loop");
    let append_done = format!("{symbol}_append_done");
    let trim_cr = format!("{symbol}_trim_cr");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let result_copy_loop = format!("{symbol}_result_copy_loop");
    let result_copy_done = format!("{symbol}_result_copy_done");
    let output_error = format!("{symbol}_output_error");
    let eof_error = format!("{symbol}_eof_error");
    let input_error = format!("{symbol}_input_error");
    let encoding_error = format!("{symbol}_encoding_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    if with_prompt {
        instructions.extend([
            abi::load_u64(abi::string_length_register(), abi::return_register(), 0),
            abi::compare_immediate(abi::string_length_register(), "0"),
            abi::branch_eq(&prompt_flush),
            abi::add_immediate(abi::string_data_register(), abi::return_register(), 8),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&output_error),
            abi::label(&prompt_flush),
            abi::move_immediate(abi::return_register(), "Integer", "1"),
        ]);
        platform.emit_sync_file(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&prompt_flush_error),
            abi::label(&prompt_ok),
        ]);
    }
    if !with_prompt {
        emit_configure_stdin_terminal(
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &terminal_slots,
            true,
            false,
            &input_error,
        )?;
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "32"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate("x10", "Integer", "32"),
        abi::store_u64("x10", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&read_loop),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&format!("{symbol}_read_eof")),
        abi::load_u8("x10", abi::stack_pointer(), BYTES_OFFSET),
        abi::compare_immediate("x10", "10"),
        abi::branch_eq(&trim_cr),
        abi::compare_immediate("x10", "127"),
        abi::branch_hi(&format!("{symbol}_multi_start")),
        abi::move_immediate("x11", "Integer", "1"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_multi_start")),
        abi::compare_immediate("x10", "194"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "223"),
        abi::branch_hi(&format!("{symbol}_line_read_third")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "2"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_third")),
        abi::compare_immediate("x10", "239"),
        abi::branch_hi(&format!("{symbol}_line_read_fourth")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "224"),
        abi::branch_ne(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("x11", "160"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_not_e0")),
        abi::compare_immediate("x10", "237"),
        abi::branch_ne(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "159"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_three_second_ok")),
        abi::label(&format!("{symbol}_line_three_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_line_three_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "3"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::branch(&have_sequence),
        abi::label(&format!("{symbol}_line_read_fourth")),
        abi::compare_immediate("x10", "240"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x10", "244"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 1),
        abi::compare_immediate("x10", "240"),
        abi::branch_ne(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("x11", "144"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_not_f0")),
        abi::compare_immediate("x10", "244"),
        abi::branch_ne(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "143"),
        abi::branch_hi(&encoding_error),
        abi::branch(&format!("{symbol}_line_four_second_ok")),
        abi::label(&format!("{symbol}_line_four_general")),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::label(&format!("{symbol}_line_four_second_ok")),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 2),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::add_immediate("x1", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&input_error),
        abi::branch_eq(&encoding_error),
        abi::load_u8("x11", abi::stack_pointer(), BYTES_OFFSET + 3),
        abi::compare_immediate("x11", "128"),
        abi::branch_lo(&encoding_error),
        abi::compare_immediate("x11", "191"),
        abi::branch_hi(&encoding_error),
        abi::move_immediate("x11", "Integer", "4"),
        abi::store_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&have_sequence),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("x12", "x10", "x11"),
        abi::load_u64("x13", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::compare_registers("x12", "x13"),
        abi::branch_gt(&grow),
        abi::branch(&grow_ok),
        abi::label(&grow),
        abi::add_registers("x14", "x13", "x13"),
        abi::compare_registers("x14", "x12"),
        abi::branch_ge(&format!("{symbol}_grow_size_ok")),
        abi::move_register("x14", "x12"),
        abi::label(&format!("{symbol}_grow_size_ok")),
        abi::store_u64("x14", abi::stack_pointer(), CAPACITY_OFFSET),
        abi::move_register(abi::return_register(), "x14"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&format!("{symbol}_grow_alloc_ok")),
        abi::branch(&alloc_error),
        abi::label(&format!("{symbol}_grow_alloc_ok")),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register("x14", "x1"),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&grow_copy_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&grow_copy_done),
        abi::load_u8("x16", "x12", 0),
        abi::store_u8("x16", "x14", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
        abi::branch(&grow_copy_loop),
        abi::label(&grow_copy_done),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::label(&grow_ok),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_registers("x12", "x12", "x10"),
        abi::add_immediate("x13", abi::stack_pointer(), BYTES_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::label(&append_loop),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&append_done),
        abi::load_u8("x14", "x13", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::subtract_immediate("x11", "x11", 1),
        abi::branch(&append_loop),
        abi::label(&append_done),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), SEQ_LEN_OFFSET),
        abi::add_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::branch(&read_loop),
        abi::label(&format!("{symbol}_read_eof")),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&eof_error),
        abi::branch(&trim_cr),
        abi::label(&trim_cr),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&format!("{symbol}_result_alloc")),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::subtract_immediate("x13", "x10", 1),
        abi::add_registers("x12", "x12", "x13"),
        abi::load_u8("x14", "x12", 0),
        abi::compare_immediate("x14", "13"),
        abi::branch_ne(&format!("{symbol}_result_alloc")),
        abi::subtract_immediate("x10", "x10", 1),
        abi::store_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::label(&format!("{symbol}_result_alloc")),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::load_u64("x12", abi::stack_pointer(), BUFFER_OFFSET),
        abi::label(&result_copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&output_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&done));
    if with_prompt {
        instructions.push(abi::label(&prompt_flush_error));
        platform.emit_errno(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate("x9", "22"),
            abi::branch_eq(&prompt_ok),
            abi::compare_immediate("x9", "45"),
            abi::branch_eq(&prompt_ok),
            abi::compare_immediate("x9", "95"),
            abi::branch_eq(&prompt_ok),
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
        ]);
        push_error_message_address(
            symbol,
            ERR_OUTPUT_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch(&done));
    }
    instructions.extend([
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&input_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::label(&done));
    if !with_prompt {
        emit_restore_stdin_terminal(
            symbol,
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &terminal_slots,
        )?;
    }
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn lower_fs_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;

    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let exists = format!("{symbol}_exists");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let alloc_symbol = ERR_ALLOCATION_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_path_exists(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&exists),
        abi::label(&missing),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_kind_exists_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    expected_kind: &str,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 288;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;
    const STAT_OFFSET: usize = 32;

    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let found = format!("{symbol}_found");
    let missing = format!("{symbol}_missing");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let alloc_symbol = ERR_ALLOCATION_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &alloc_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: alloc_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
        abi::add_immediate("x1", abi::stack_pointer(), STAT_OFFSET),
    ]);
    platform.emit_path_stat(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&missing),
        abi::load_u16(
            "x9",
            abi::stack_pointer(),
            STAT_OFFSET + platform.stat_mode_offset(),
        ),
        abi::move_immediate("x10", "Integer", FS_MODE_TYPE_MASK),
        abi::and_registers("x9", "x9", "x10"),
        abi::move_immediate("x10", "Integer", expected_kind),
        abi::compare_registers("x9", "x10"),
        abi::branch_eq(&found),
        abi::label(&missing),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_current_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const LENGTH_OFFSET: usize = 16;
    const GETCWD_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", GETCWD_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register(abi::return_register(), "x1"),
        abi::move_immediate("x1", "Integer", GETCWD_CAPACITY),
    ]);
    platform.emit_current_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x10", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register("x11", "x10"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8("x13", "x11", 0),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::store_u64("x12", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x12", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x13", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_temp_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const LENGTH_OFFSET: usize = 16;
    const TEMP_CAPACITY: &str = "4096";

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", TEMP_CAPACITY),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_register(abi::return_register(), "x1"),
        abi::move_immediate("x1", "Integer", TEMP_CAPACITY),
    ]);
    platform.emit_temp_directory(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x13", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_path_operation_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    operation: FsPathOperation,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let call_error = format!("{symbol}_call_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid_path),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        operation,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&call_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        false,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&invalid_path),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_create_directories_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 64;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const ALLOC_OFFSET: usize = 16;
    const CURSOR_OFFSET: usize = 24;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid_path = format!("{symbol}_invalid_path");
    let scan_loop = format!("{symbol}_scan_loop");
    let mkdir_prefix = format!("{symbol}_mkdir_prefix");
    let prefix_ok = format!("{symbol}_prefix_ok");
    let final_mkdir = format!("{symbol}_final_mkdir");
    let final_ok = format!("{symbol}_final_ok");
    let call_error = format!("{symbol}_call_error");
    let err_not_found = format!("{symbol}_err_not_found");
    let err_access_denied = format!("{symbol}_err_access_denied");
    let err_output = format!("{symbol}_err_output");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid_path),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid_path),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x10", abi::stack_pointer(), ALLOC_OFFSET),
        abi::load_u8("x11", "x10", 0),
        abi::compare_immediate("x11", "47"),
        abi::branch_ne(&scan_loop),
        abi::add_immediate("x10", "x10", 1),
        abi::label(&scan_loop),
        abi::store_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u8("x11", "x10", 0),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&final_mkdir),
        abi::compare_immediate("x11", "47"),
        abi::branch_eq(&mkdir_prefix),
        abi::add_immediate("x10", "x10", 1),
        abi::branch(&scan_loop),
        abi::label(&mkdir_prefix),
        abi::store_u8("x31", "x10", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Mkdir,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_immediate("x11", "Integer", "47"),
        abi::store_u8("x11", "x10", 0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&prefix_ok),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", "17"),
        abi::branch_ne(&call_error),
        abi::label(&prefix_ok),
        abi::load_u64("x10", abi::stack_pointer(), CURSOR_OFFSET),
        abi::add_immediate("x10", "x10", 1),
        abi::branch(&scan_loop),
        abi::label(&final_mkdir),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ALLOC_OFFSET),
    ]);
    platform.emit_fs_path_operation(
        symbol,
        FsPathOperation::Mkdir,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&final_ok),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&final_ok),
        abi::branch(&call_error),
        abi::label(&final_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&call_error),
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::branch(&err_output),
        abi::label(&invalid_path),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&err_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ACCESS_DENIED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_list_directory_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const DIR_OFFSET: usize = 24;
    const COUNT_OFFSET: usize = 32;
    const DATA_LEN_OFFSET: usize = 40;
    const COLLECTION_OFFSET: usize = 48;
    const ENTRY_CURSOR_OFFSET: usize = 56;
    const DATA_CURSOR_OFFSET: usize = 64;
    const DATA_OFFSET_OFFSET: usize = 72;

    let path_alloc_ok = format!("{symbol}_path_alloc_ok");
    let path_copy_loop = format!("{symbol}_path_copy_loop");
    let path_copy_done = format!("{symbol}_path_copy_done");
    let first_open_ok = format!("{symbol}_first_open_ok");
    let count_loop = format!("{symbol}_count_loop");
    let count_done = format!("{symbol}_count_done");
    let count_skip = format!("{symbol}_count_skip");
    let second_open_ok = format!("{symbol}_second_open_ok");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_done = format!("{symbol}_fill_done");
    let fill_skip = format!("{symbol}_fill_skip");
    let copy_name_loop = format!("{symbol}_copy_name_loop");
    let copy_name_done = format!("{symbol}_copy_name_done");
    let invalid = format!("{symbol}_invalid");
    let open_error = format!("{symbol}_open_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let name_offset = platform.dirent_name_offset();
    let namlen_offset = platform.dirent_name_length_offset();
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&path_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&path_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&path_copy_loop),
        abi::label(&path_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
    ]);
    platform.emit_opendir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&first_open_ok),
        abi::branch(&open_error),
        abi::label(&first_open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::label(&count_loop),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if platform.target() == "linux-aarch64" {
        let name_len_loop = format!("{symbol}_count_name_len_loop");
        let name_len_done = format!("{symbol}_count_name_len_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&count_done),
            abi::add_immediate("x11", abi::return_register(), name_offset),
            abi::move_register("x13", "x11"),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8("x12", "x13", 0),
            abi::compare_immediate("x12", "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate("x10", "x10", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
    } else {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&count_done),
            abi::load_u16("x10", abi::return_register(), namlen_offset),
            abi::add_immediate("x11", abi::return_register(), name_offset),
        ]);
    }
    instructions.extend([
        abi::compare_immediate("x10", "1"),
        abi::branch_ne(&count_skip),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_skip),
        abi::compare_immediate("x10", "2"),
        abi::branch_ne(&count_skip.replace("skip", "keep")),
    ]);
    let count_keep = count_skip.replace("skip", "keep");
    instructions.extend([
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_ne(&count_keep),
        abi::load_u8("x12", "x11", 1),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&count_loop),
        abi::label(&count_keep),
        abi::load_u64("x12", abi::stack_pointer(), COUNT_OFFSET),
        abi::add_immediate("x12", "x12", 1),
        abi::store_u64("x12", abi::stack_pointer(), COUNT_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::add_registers("x12", "x12", "x10"),
        abi::store_u64("x12", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x10", "x11"),
        abi::load_u64("x13", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::add_registers("x12", "x12", "x13"),
        abi::add_immediate(abi::return_register(), "x12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), COLLECTION_OFFSET),
        abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_STRING.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x9", "Byte", "1"),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::load_u64("x11", abi::stack_pointer(), DATA_LEN_OFFSET),
        abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x12", "x1", COLLECTION_HEADER_SIZE),
        abi::store_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::move_immediate("x13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x14", "x10", "x13"),
        abi::add_registers("x12", "x12", "x14"),
        abi::store_u64("x12", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
    ]);
    platform.emit_opendir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_gt(&second_open_ok),
        abi::branch(&open_error),
        abi::label(&second_open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_readdir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if platform.target() == "linux-aarch64" {
        let name_len_loop = format!("{symbol}_fill_name_len_loop");
        let name_len_done = format!("{symbol}_fill_name_len_done");
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fill_done),
            abi::add_immediate("x11", abi::return_register(), name_offset),
            abi::move_register("x13", "x11"),
            abi::move_immediate("x10", "Integer", "0"),
            abi::label(&name_len_loop),
            abi::load_u8("x12", "x13", 0),
            abi::compare_immediate("x12", "0"),
            abi::branch_eq(&name_len_done),
            abi::add_immediate("x10", "x10", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&name_len_loop),
            abi::label(&name_len_done),
        ]);
    } else {
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&fill_done),
            abi::load_u16("x10", abi::return_register(), namlen_offset),
            abi::add_immediate("x11", abi::return_register(), name_offset),
        ]);
    }
    instructions.extend([
        abi::compare_immediate("x10", "1"),
        abi::branch_ne(&fill_skip),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_skip),
    ]);
    let fill_keep = fill_skip.replace("skip", "keep");
    instructions.extend([
        abi::compare_immediate("x10", "2"),
        abi::branch_ne(&fill_keep),
        abi::load_u8("x12", "x11", 0),
        abi::compare_immediate("x12", "46"),
        abi::branch_ne(&fill_keep),
        abi::load_u8("x12", "x11", 1),
        abi::compare_immediate("x12", "46"),
        abi::branch_eq(&fill_loop),
        abi::label(&fill_keep),
        abi::load_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::move_immediate("x13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x13", "x12", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::load_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::store_u64("x13", "x12", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::store_u64("x10", "x12", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("x14", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&copy_name_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&copy_name_done),
        abi::load_u8("x16", "x11", 0),
        abi::store_u8("x16", "x14", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
        abi::branch(&copy_name_loop),
        abi::label(&copy_name_done),
        abi::store_u64("x14", abi::stack_pointer(), DATA_CURSOR_OFFSET),
        abi::load_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::add_registers("x13", "x13", "x10"),
        abi::store_u64("x13", abi::stack_pointer(), DATA_OFFSET_OFFSET),
        abi::load_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE),
        abi::store_u64("x12", abi::stack_pointer(), ENTRY_CURSOR_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
    ]);
    platform.emit_closedir(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        COLLECTION_OFFSET,
    ));
    instructions.push(abi::branch_link(SORT_STRING_LIST_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: SORT_STRING_LIST_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            COLLECTION_OFFSET,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_create_temp_file_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const DIR_OFFSET: usize = 8;
    const PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const FILE_OFFSET: usize = 32;
    const RANDOM_OFFSET: usize = 48;
    const CURSOR_OFFSET: usize = 64;
    const UUID_FILE_EXTRA: usize = 46;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_dir = format!("{symbol}_copy_dir");
    let copy_done = format!("{symbol}_copy_done");
    let random_ok = format!("{symbol}_random_ok");
    let fd_ok = format!("{symbol}_fd_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DIR_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", UUID_FILE_EXTRA),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), PATH_OFFSET),
        abi::move_register("x13", "x1"),
        abi::load_u64("x9", abi::stack_pointer(), DIR_OFFSET),
        abi::load_u64("x10", "x9", 0),
        abi::add_immediate("x11", "x9", 8),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&copy_dir),
        abi::compare_registers("x12", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::compare_immediate("x14", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x14", "x13", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&copy_dir),
        abi::label(&copy_done),
    ]);
    for byte in b"/mfb-" {
        instructions.extend([
            abi::move_immediate("x14", "Byte", &byte.to_string()),
            abi::store_u8("x14", "x13", 0),
            abi::add_immediate("x13", "x13", 1),
        ]);
    }
    instructions.extend([
        abi::store_u64("x13", abi::stack_pointer(), CURSOR_OFFSET),
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), RANDOM_OFFSET),
        abi::move_immediate("x1", "Integer", "16"),
    ]);
    platform.emit_random_bytes(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&random_ok),
        abi::branch(&open_error),
        abi::label(&random_ok),
        abi::load_u64("x13", abi::stack_pointer(), CURSOR_OFFSET),
    ]);
    emit_uuid_v4_to_path(symbol, &mut instructions, RANDOM_OFFSET, "x13");
    for byte in b".tmp" {
        instructions.extend([
            abi::move_immediate("x14", "Byte", &byte.to_string()),
            abi::store_u8("x14", "x13", 0),
            abi::add_immediate("x13", "x13", 1),
        ]);
    }
    instructions.extend([
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::move_immediate("x1", "Integer", temp_file_open_flags(platform.target())),
        abi::move_immediate("x2", "Integer", "384"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&fd_ok),
        abi::branch(&open_error),
        abi::label(&fd_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn temp_file_open_flags(target: &str) -> &'static str {
    match target {
        "linux-aarch64" => "524482",
        _ => "2562",
    }
}

fn emit_uuid_v4_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    random_offset: usize,
    cursor: &str,
) {
    for index in 0..16 {
        if matches!(index, 4 | 6 | 8 | 10) {
            instructions.extend([
                abi::move_immediate("x14", "Byte", "45"),
                abi::store_u8("x14", cursor, 0),
                abi::add_immediate(cursor, cursor, 1),
            ]);
        }
        instructions.push(abi::load_u8(
            "x9",
            abi::stack_pointer(),
            random_offset + index,
        ));
        if index == 6 {
            instructions.extend([
                abi::move_immediate("x10", "Integer", "15"),
                abi::and_registers("x9", "x9", "x10"),
                abi::move_immediate("x10", "Integer", "64"),
                abi::or_registers("x9", "x9", "x10"),
            ]);
        } else if index == 8 {
            instructions.extend([
                abi::move_immediate("x10", "Integer", "63"),
                abi::and_registers("x9", "x9", "x10"),
                abi::move_immediate("x10", "Integer", "128"),
                abi::or_registers("x9", "x9", "x10"),
            ]);
        }
        instructions.extend([
            abi::shift_right_immediate("x10", "x9", 4),
            abi::move_immediate("x11", "Integer", "15"),
            abi::and_registers("x11", "x9", "x11"),
        ]);
        emit_hex_nibble_to_path(symbol, instructions, index, "high", "x10", cursor);
        emit_hex_nibble_to_path(symbol, instructions, index, "low", "x11", cursor);
    }
}

fn emit_hex_nibble_to_path(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    byte_index: usize,
    half: &str,
    nibble: &str,
    cursor: &str,
) {
    let digit = format!("{symbol}_uuid_{byte_index}_{half}_digit");
    let store = format!("{symbol}_uuid_{byte_index}_{half}_store");
    instructions.extend([
        abi::compare_immediate(nibble, "10"),
        abi::branch_lt(&digit),
        abi::add_immediate("x12", nibble, 87),
        abi::branch(&store),
        abi::label(&digit),
        abi::add_immediate("x12", nibble, 48),
        abi::label(&store),
        abi::store_u8("x12", cursor, 0),
        abi::add_immediate(cursor, cursor, 1),
    ]);
}

fn lower_fs_open_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    no_follow: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const MODE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FLAGS_OFFSET: usize = 32;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let read = format!("{symbol}_mode_read");
    let write = format!("{symbol}_mode_write");
    let read_write = format!("{symbol}_mode_read_write");
    let append = format!("{symbol}_mode_append");
    let flags_done = format!("{symbol}_flags_done");
    let open_ok = format!("{symbol}_open_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let open_error = format!("{symbol}_open_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), no_follow);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), MODE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x9", abi::stack_pointer(), MODE_OFFSET),
        abi::load_u64("x10", "x9", 0),
    ]);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"r", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"read", &read, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"w", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"write", &write, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"rw", &read_write, symbol);
    emit_branch_if_ascii_literal(
        &mut instructions,
        "x9",
        "x10",
        b"readWrite",
        &read_write,
        symbol,
    );
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"a", &append, symbol);
    emit_branch_if_ascii_literal(&mut instructions, "x9", "x10", b"append", &append, symbol);
    instructions.extend([
        abi::branch(&invalid),
        abi::label(&read),
        abi::move_immediate("x11", "Integer", flags.read),
        abi::branch(&flags_done),
        abi::label(&write),
        abi::move_immediate("x11", "Integer", flags.write),
        abi::branch(&flags_done),
        abi::label(&read_write),
        abi::move_immediate("x11", "Integer", flags.read_write),
        abi::branch(&flags_done),
        abi::label(&append),
        abi::move_immediate("x11", "Integer", flags.append),
        abi::label(&flags_done),
        abi::store_u64("x11", abi::stack_pointer(), FLAGS_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_register("x1", "x11"),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FLAGS_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&file_alloc_ok),
        abi::load_u64("x9", abi::stack_pointer(), FLAGS_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        no_follow,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 16;
    let already_closed = format!("{symbol}_already_closed");
    let close_error = format!("{symbol}_close_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&already_closed),
        abi::load_u64(
            abi::return_register(),
            abi::return_register(),
            FILE_OFFSET_FD,
        ),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate("x9", "Integer", "1"),
        abi::load_u64("x10", abi::stack_pointer(), 8),
        abi::store_u64("x9", "x10", FILE_OFFSET_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&already_closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_CLOSE_FAILED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_CLOSE_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_write_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const FD_OFFSET: usize = 24;
    const REMAINING_OFFSET: usize = 32;
    const CURSOR_OFFSET: usize = 40;
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::store_u64("x1", abi::stack_pointer(), 16),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::load_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&loop_label),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&done_write),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_read_all_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const FILE_OFFSET: usize = 8;
    const FD_OFFSET: usize = 16;
    const START_OFFSET: usize = 24;
    const END_OFFSET: usize = 32;
    const LEN_OFFSET: usize = 40;
    const STRING_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&seek_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), STRING_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&read_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64("x0", abi::stack_pointer(), STRING_OFFSET),
        abi::load_u64("x1", "x0", 0),
        abi::add_immediate("x0", "x0", 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STRING_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_write_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const FD_OFFSET: usize = 24;
    const REMAINING_OFFSET: usize = 32;
    const CURSOR_OFFSET: usize = 40;
    let loop_label = format!("{symbol}_write_loop");
    let done_write = format!("{symbol}_write_done");
    let closed = format!("{symbol}_closed");
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), 8),
        abi::store_u64("x1", abi::stack_pointer(), 16),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::load_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::load_u64("x12", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x13", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x12", "x13"),
        abi::add_registers("x11", "x11", "x12"),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&loop_label),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&done_write),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&loop_label),
        abi::label(&done_write),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_read_all_bytes_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 112;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    const LEN_OFFSET: usize = 32;
    const COLLECTION_OFFSET: usize = 40;
    const DATA_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_lt(&seek_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x10", "x11"),
        abi::add_immediate("x12", "x12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "x12", "x10"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), COLLECTION_OFFSET),
        abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x9", "Byte", "1"),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x13", "x10", "x12"),
        abi::add_registers("x14", "x11", "x13"),
        abi::store_u64("x14", abi::stack_pointer(), DATA_OFFSET),
        abi::move_immediate("x15", "Integer", "0"),
        abi::label(&entry_loop),
        abi::compare_registers("x15", "x10"),
        abi::branch_eq(&entry_done),
        abi::move_immediate("x16", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x16", "x11", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x15", "x11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("x16", "Integer", "1"),
        abi::store_u64("x16", "x11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x15", "x15", 1),
        abi::branch(&entry_loop),
        abi::label(&entry_done),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), DATA_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&read_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            COLLECTION_OFFSET,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_eof_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let at_eof = format!("{symbol}_at_eof");
    let not_eof = format!("{symbol}_not_eof");
    let done = format!("{symbol}_done");
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), START_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), END_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_ge(&at_eof),
        abi::branch(&not_eof),
        abi::label(&at_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_eof),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_canonical_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const BUFFER_OFFSET: usize = 24;
    const LENGTH_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const PATH_MAX_PLUS_NUL: usize = 4097;

    let path_alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let buffer_alloc_ok = format!("{symbol}_buffer_alloc_ok");
    let realpath_ok = format!("{symbol}_realpath_ok");
    let length_loop = format!("{symbol}_length_loop");
    let length_done = format!("{symbol}_length_done");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let result_copy_loop = format!("{symbol}_result_copy_loop");
    let result_copy_done = format!("{symbol}_result_copy_done");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let realpath_error = format!("{symbol}_realpath_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&path_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&path_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&realpath_ok),
        abi::load_u64("x10", abi::stack_pointer(), BUFFER_OFFSET),
        abi::move_immediate("x11", "Integer", "0"),
        abi::label(&length_loop),
        abi::add_registers("x12", "x10", "x11"),
        abi::load_u8("x13", "x12", 0),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&length_done),
        abi::add_immediate("x11", "x11", 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::store_u64("x11", abi::stack_pointer(), LENGTH_OFFSET),
        abi::add_immediate(abi::return_register(), "x11", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LENGTH_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::load_u64("x11", abi::stack_pointer(), BUFFER_OFFSET),
        abi::add_immediate("x12", "x1", 8),
        abi::label(&result_copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&result_copy_done),
        abi::load_u8("x13", "x11", 0),
        abi::store_u8("x13", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&result_copy_loop),
        abi::label(&result_copy_done),
        abi::store_u8("x31", "x12", 0),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&realpath_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_is_within_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 112;
    const LR_OFFSET: usize = 0;
    const BASE_OFFSET: usize = 8;
    const CHILD_OFFSET: usize = 16;
    const C_BASE_OFFSET: usize = 24;
    const C_CHILD_OFFSET: usize = 32;
    const BASE_BUFFER_OFFSET: usize = 40;
    const CHILD_BUFFER_OFFSET: usize = 48;
    const PATH_MAX_PLUS_NUL: usize = 4097;

    let base_alloc_ok = format!("{symbol}_base_alloc_ok");
    let child_alloc_ok = format!("{symbol}_child_alloc_ok");
    let base_copy_loop = format!("{symbol}_base_copy_loop");
    let base_copy_done = format!("{symbol}_base_copy_done");
    let child_copy_loop = format!("{symbol}_child_copy_loop");
    let child_copy_done = format!("{symbol}_child_copy_done");
    let base_buffer_alloc_ok = format!("{symbol}_base_buffer_alloc_ok");
    let child_buffer_alloc_ok = format!("{symbol}_child_buffer_alloc_ok");
    let base_realpath_ok = format!("{symbol}_base_realpath_ok");
    let child_realpath_ok = format!("{symbol}_child_realpath_ok");
    let root_true = format!("{symbol}_root_true");
    let compare_loop = format!("{symbol}_compare_loop");
    let base_ended = format!("{symbol}_base_ended");
    let true_label = format!("{symbol}_true");
    let false_label = format!("{symbol}_false");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let realpath_error = format!("{symbol}_realpath_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), BASE_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), CHILD_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_BASE_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), BASE_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&base_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&base_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&base_copy_loop),
        abi::label(&base_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x9", abi::stack_pointer(), CHILD_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_CHILD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), CHILD_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&child_copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&child_copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&child_copy_loop),
        abi::label(&child_copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&base_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&base_buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), BASE_BUFFER_OFFSET),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &PATH_MAX_PLUS_NUL.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&child_buffer_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&child_buffer_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_BASE_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BASE_BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&base_realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&base_realpath_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_CHILD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
    ]);
    platform.emit_realpath(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&child_realpath_ok),
        abi::branch(&realpath_error),
        abi::label(&child_realpath_ok),
        abi::load_u64("x10", abi::stack_pointer(), BASE_BUFFER_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), CHILD_BUFFER_OFFSET),
        abi::load_u8("x12", "x10", 0),
        abi::compare_immediate("x12", "47"),
        abi::branch_ne(&compare_loop),
        abi::load_u8("x12", "x10", 1),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&root_true),
        abi::label(&compare_loop),
        abi::load_u8("x12", "x10", 0),
        abi::load_u8("x13", "x11", 0),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&base_ended),
        abi::compare_registers("x12", "x13"),
        abi::branch_ne(&false_label),
        abi::add_immediate("x10", "x10", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::branch(&compare_loop),
        abi::label(&base_ended),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&true_label),
        abi::compare_immediate("x13", "47"),
        abi::branch_eq(&true_label),
        abi::branch(&false_label),
        abi::label(&root_true),
        abi::label(&true_label),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&false_label),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&realpath_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

#[derive(Clone, Copy)]
enum AtomicWriteValueKind {
    String,
    Bytes,
}

fn lower_fs_atomic_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    value_kind: AtomicWriteValueKind,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const TEMP_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;
    const C_TEMP_OFFSET: usize = 56;
    const C_FINAL_OFFSET: usize = 64;
    const TEMPLATE_SUFFIX: &[u8] = b".mfb-XXXXXX.tmp";
    const MFB_PREFIX: &[u8] = b".mfb-";
    const X_MARKERS: &[u8] = b"XXXXXX";
    const TMP_SUFFIX: &[u8] = b".tmp";
    const MKTEMPS_SUFFIX_LEN: usize = TMP_SUFFIX.len();

    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let copy_path_loop = format!("{symbol}_copy_path_loop");
    let copy_path_done = format!("{symbol}_copy_path_done");
    let mkstemps_ok = format!("{symbol}_mkstemps_ok");
    let write_loop = format!("{symbol}_write_loop");
    let write_ok = format!("{symbol}_write_ok");
    let write_error = format!("{symbol}_write_error");
    let sync_error = format!("{symbol}_sync_error");
    let close_error = format!("{symbol}_close_error");
    let c_temp_alloc_ok = format!("{symbol}_c_temp_alloc_ok");
    let c_final_alloc_ok = format!("{symbol}_c_final_alloc_ok");
    let c_temp_loop = format!("{symbol}_c_temp_loop");
    let c_temp_done = format!("{symbol}_c_temp_done");
    let c_final_loop = format!("{symbol}_c_final_loop");
    let c_final_done = format!("{symbol}_c_final_done");
    let rename_ok = format!("{symbol}_rename_ok");
    let invalid = format!("{symbol}_invalid");
    let alloc_error = format!("{symbol}_alloc_error");
    let rename_error = format!("{symbol}_rename_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 9 + TEMPLATE_SUFFIX.len()),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::move_immediate("x12", "Integer", &(TEMPLATE_SUFFIX.len()).to_string()),
        abi::add_registers("x13", "x11", "x12"),
        abi::store_u64("x13", "x1", 0),
        abi::add_immediate("x14", "x10", 8),
        abi::add_immediate("x15", "x1", 8),
        abi::move_immediate("x16", "Integer", "0"),
        abi::label(&copy_path_loop),
        abi::compare_registers("x16", "x11"),
        abi::branch_eq(&copy_path_done),
        abi::load_u8("x17", "x14", 0),
        abi::compare_immediate("x17", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x17", "x15", 0),
        abi::add_immediate("x14", "x14", 1),
        abi::add_immediate("x15", "x15", 1),
        abi::add_immediate("x16", "x16", 1),
        abi::branch(&copy_path_loop),
        abi::label(&copy_path_done),
    ]);
    for byte in MFB_PREFIX {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    for byte in X_MARKERS {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    for byte in TMP_SUFFIX {
        instructions.extend([
            abi::move_immediate("x17", "Byte", &byte.to_string()),
            abi::store_u8("x17", "x15", 0),
            abi::add_immediate("x15", "x15", 1),
        ]);
    }
    instructions.extend([
        abi::store_u8("x31", "x15", 0),
        abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            TEMP_PATH_OFFSET,
        ),
        abi::add_immediate(abi::return_register(), abi::return_register(), 8),
        abi::move_immediate("x1", "Integer", &MKTEMPS_SUFFIX_LEN.to_string()),
    ]);
    platform.emit_mkstemps(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&mkstemps_ok),
        abi::branch(&rename_error),
        abi::label(&mkstemps_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    match value_kind {
        AtomicWriteValueKind::String => {
            instructions.extend([
                abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64("x11", "x10", 0),
                abi::add_immediate("x12", "x10", 8),
            ]);
        }
        AtomicWriteValueKind::Bytes => {
            instructions.extend([
                abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
                abi::load_u64("x11", "x10", COLLECTION_OFFSET_DATA_LENGTH),
                abi::add_immediate("x12", "x10", COLLECTION_HEADER_SIZE),
                abi::load_u64("x13", "x10", COLLECTION_OFFSET_CAPACITY),
                abi::move_immediate("x14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
                abi::multiply_registers("x13", "x13", "x14"),
                abi::add_registers("x12", "x12", "x13"),
            ]);
        }
    }
    instructions.extend([
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_ok),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&sync_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::load_u64("x9", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&c_temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_TEMP_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64(abi::return_register(), "x9", 0),
        abi::add_immediate(abi::return_register(), abi::return_register(), 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&c_final_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&c_final_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_FINAL_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), TEMP_PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::load_u64("x13", abi::stack_pointer(), C_TEMP_OFFSET),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&c_temp_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&c_temp_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&c_temp_loop),
        abi::label(&c_temp_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::load_u64("x13", abi::stack_pointer(), C_FINAL_OFFSET),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&c_final_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&c_final_done),
        abi::load_u8("x15", "x12", 0),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&c_final_loop),
        abi::label(&c_final_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_TEMP_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), C_FINAL_OFFSET),
    ]);
    platform.emit_rename_path(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&rename_ok),
        abi::branch(&rename_error),
        abi::label(&rename_ok),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&rename_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&write_error),
        abi::label(&sync_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_write_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;
    const CLOSE_STATUS_OFFSET: usize = 56;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_error = format!("{symbol}_write_error");
    let close_error = format!("{symbol}_close_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mode_flags = if append { flags.append } else { flags.write };
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", mode_flags),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            CLOSE_STATUS_OFFSET,
        ),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_read_text_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const END_OFFSET: usize = 32;
    const LEN_OFFSET: usize = 40;
    const STRING_OFFSET: usize = 48;
    const REMAINING_OFFSET: usize = 56;
    const CURSOR_OFFSET: usize = 64;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let seek_error = format!("{symbol}_seek_error");
    let string_alloc_ok = format!("{symbol}_string_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let close_and_read_error = format!("{symbol}_close_and_read_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", flags.read),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_register("x9", abi::return_register()),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_and_read_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_and_read_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&string_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&string_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), STRING_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&read_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let encoding_error = format!("{symbol}_encoding_error");
    instructions.extend([
        abi::load_u64("x0", abi::stack_pointer(), STRING_OFFSET),
        abi::add_immediate("x0", "x0", 8),
        abi::load_u64("x1", abi::stack_pointer(), LEN_OFFSET),
    ]);
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STRING_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&read_error),
        abi::label(&close_and_read_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&seek_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        false,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_write_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    append: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 80;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const VALUE_OFFSET: usize = 16;
    const C_PATH_OFFSET: usize = 24;
    const FD_OFFSET: usize = 32;
    const REMAINING_OFFSET: usize = 40;
    const CURSOR_OFFSET: usize = 48;

    let alloc_ok = format!("{symbol}_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_error = format!("{symbol}_write_error");
    let close_error = format!("{symbol}_close_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mode_flags = if append { flags.append } else { flags.write };
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", mode_flags),
        abi::move_immediate("x2", "Integer", "438"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), VALUE_OFFSET),
        abi::load_u64("x11", "x10", COLLECTION_OFFSET_DATA_LENGTH),
        abi::add_immediate("x12", "x10", COLLECTION_HEADER_SIZE),
        abi::load_u64("x13", "x10", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x13", "x13", "x14"),
        abi::add_registers("x12", "x12", "x13"),
        abi::store_u64("x11", abi::stack_pointer(), REMAINING_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_sync_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&close_error),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([abi::branch(&done), abi::label(&open_error)]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&close_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_OUTPUT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_read_bytes_path_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const PATH_OFFSET: usize = 8;
    const C_PATH_OFFSET: usize = 16;
    const FD_OFFSET: usize = 24;
    const FILE_OFFSET: usize = 32;
    const RESULT_TAG_OFFSET: usize = 48;
    const RESULT_VALUE_OFFSET: usize = 56;
    const RESULT_MESSAGE_OFFSET: usize = 64;

    let alloc_ok = format!("{symbol}_path_alloc_ok");
    let file_alloc_ok = format!("{symbol}_file_alloc_ok");
    let copy_loop = format!("{symbol}_path_copy_loop");
    let copy_done = format!("{symbol}_path_copy_done");
    let invalid = format!("{symbol}_invalid");
    let open_ok = format!("{symbol}_open_ok");
    let open_error = format!("{symbol}_open_error");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let flags = open_flag_set(platform.target(), false);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x9", abi::return_register(), 0),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&invalid),
        abi::add_immediate(abi::return_register(), "x9", 1),
        abi::move_immediate("x1", "Integer", "1"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), C_PATH_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PATH_OFFSET),
        abi::load_u64("x11", "x10", 0),
        abi::add_immediate("x12", "x10", 8),
        abi::move_register("x13", "x1"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x14", "x11"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x15", "x12", 0),
        abi::compare_immediate("x15", "0"),
        abi::branch_eq(&invalid),
        abi::store_u8("x15", "x13", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x13", 0),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), C_PATH_OFFSET),
        abi::move_immediate("x1", "Integer", flags.read),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_open_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge(&open_ok),
        abi::branch(&open_error),
        abi::label(&open_ok),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&file_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&file_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), FILE_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
        abi::store_u64("x31", "x1", FILE_OFFSET_STATE),
        abi::move_register(abi::return_register(), "x1"),
        abi::branch_link("_mfb_rt_fs_fs_readAllBytes"),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: "_mfb_rt_fs_fs_readAllBytes".to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), RESULT_TAG_OFFSET),
        abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            RESULT_VALUE_OFFSET,
        ),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            RESULT_MESSAGE_OFFSET,
        ),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_close_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), RESULT_TAG_OFFSET),
        abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            RESULT_VALUE_OFFSET,
        ),
        abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            RESULT_MESSAGE_OFFSET,
        ),
        abi::branch(&done),
        abi::label(&open_error),
    ]);
    platform.emit_errno(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fs_path_errno_error_mapping(
        symbol,
        platform.target(),
        false,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&invalid),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

fn lower_fs_read_line_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 128;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const START_OFFSET: usize = 16;
    const END_OFFSET: usize = 24;
    const LEN_OFFSET: usize = 32;
    const TEMP_OFFSET: usize = 40;
    const REMAINING_OFFSET: usize = 48;
    const CURSOR_OFFSET: usize = 56;
    const LINE_LEN_OFFSET: usize = 64;
    const CONSUMED_OFFSET: usize = 72;
    const RESULT_OFFSET: usize = 80;

    let closed = format!("{symbol}_closed");
    let seek_error = format!("{symbol}_seek_error");
    let eof_error = format!("{symbol}_eof_error");
    let temp_alloc_ok = format!("{symbol}_temp_alloc_ok");
    let read_loop = format!("{symbol}_read_loop");
    let read_done = format!("{symbol}_read_done");
    let read_error = format!("{symbol}_read_error");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_no_newline = format!("{symbol}_scan_no_newline");
    let scan_newline = format!("{symbol}_scan_newline");
    let trim_done = format!("{symbol}_trim_done");
    let result_alloc_ok = format!("{symbol}_result_alloc_ok");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "1"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), START_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "2"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), END_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), START_OFFSET),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), END_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), START_OFFSET),
        abi::compare_registers("x10", "x11"),
        abi::branch_le(&eof_error),
        abi::subtract_registers("x10", "x10", "x11"),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&temp_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&temp_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), TEMP_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_immediate("x11", "x1", 8),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::label(&read_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), CURSOR_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_read_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&read_error),
        abi::load_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), CURSOR_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64("x10", abi::stack_pointer(), TEMP_OFFSET),
        abi::add_immediate("x11", "x10", 8),
        abi::load_u64("x12", abi::stack_pointer(), LEN_OFFSET),
        abi::move_immediate("x13", "Integer", "0"),
        abi::move_immediate("x14", "Integer", "0"),
        abi::label(&scan_loop),
        abi::compare_immediate("x12", "0"),
        abi::branch_eq(&scan_no_newline),
        abi::load_u8("x15", "x11", 0),
        abi::add_immediate("x14", "x14", 1),
        abi::compare_immediate("x15", "10"),
        abi::branch_eq(&scan_newline),
        abi::add_immediate("x13", "x13", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::subtract_immediate("x12", "x12", 1),
        abi::branch(&scan_loop),
        abi::label(&scan_no_newline),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x13", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::branch(&trim_done),
        abi::label(&scan_newline),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x14", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::compare_immediate("x13", "0"),
        abi::branch_eq(&trim_done),
        abi::subtract_immediate("x16", "x11", 1),
        abi::load_u8("x15", "x16", 0),
        abi::compare_immediate("x15", "13"),
        abi::branch_ne(&trim_done),
        abi::subtract_immediate("x13", "x13", 1),
        abi::store_u64("x13", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::label(&trim_done),
        abi::load_u64("x10", abi::stack_pointer(), START_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), CONSUMED_OFFSET),
        abi::add_registers("x1", "x10", "x11"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_seek_file(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&seek_error),
        abi::load_u64("x10", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), LINE_LEN_OFFSET),
        abi::store_u64("x10", "x1", 0),
        abi::add_immediate("x11", "x1", 8),
        abi::load_u64("x12", abi::stack_pointer(), TEMP_OFFSET),
        abi::add_immediate("x12", "x12", 8),
        abi::label(&copy_loop),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x13", "x12", 0),
        abi::store_u8("x13", "x11", 0),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x11", "x11", 1),
        abi::subtract_immediate("x10", "x10", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x11", 0),
        abi::load_u64("x0", abi::stack_pointer(), RESULT_OFFSET),
        abi::load_u64("x1", "x0", 0),
        abi::add_immediate("x0", "x0", 8),
    ]);
    let encoding_error = format!("{symbol}_encoding_error");
    emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&encoding_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ENCODING_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ENCODING_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&closed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_RESOURCE_CLOSED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
        abi::label(&eof_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_EOF_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_EOF_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&seek_error),
        abi::label(&read_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_READ_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_READ_SYMBOL, &mut instructions, &mut relocations);
    instructions.extend([
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);

    Ok((
        CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
    ))
}

struct OpenFlagSet {
    read: &'static str,
    write: &'static str,
    read_write: &'static str,
    append: &'static str,
}

fn open_flag_set(target: &str, no_follow: bool) -> OpenFlagSet {
    match (target, no_follow) {
        ("linux-aarch64", false) => OpenFlagSet {
            read: "0",
            write: "577",
            read_write: "66",
            append: "1089",
        },
        ("linux-aarch64", true) => OpenFlagSet {
            read: "32768",
            write: "33345",
            read_write: "32834",
            append: "33857",
        },
        (_, false) => OpenFlagSet {
            read: "0",
            write: "1537",
            read_write: "514",
            append: "521",
        },
        (_, true) => OpenFlagSet {
            read: "256",
            write: "1793",
            read_write: "770",
            append: "777",
        },
    }
}

fn emit_branch_if_ascii_literal(
    instructions: &mut Vec<CodeInstruction>,
    ptr: &str,
    len: &str,
    literal: &[u8],
    target: &str,
    symbol: &str,
) {
    let next = format!(
        "{symbol}_literal_{}_{}",
        target.rsplit('_').next().unwrap_or("next"),
        literal.len()
    );
    instructions.extend([
        abi::compare_immediate(len, &literal.len().to_string()),
        abi::branch_ne(&next),
    ]);
    for (index, byte) in literal.iter().enumerate() {
        instructions.extend([
            abi::load_u8("x12", ptr, 8 + index),
            abi::compare_immediate("x12", &byte.to_string()),
            abi::branch_ne(&next),
        ]);
    }
    instructions.extend([abi::branch(target), abi::label(&next)]);
}

fn emit_errno_error_mapping(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let err_not_found = format!("{symbol}_errno_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_output = format!("{symbol}_errno_output");
    instructions.extend([
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&err_already_exists),
        abi::branch(&err_output),
        abi::label(&err_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
}

/// Filesystem-context errno mapping for path-based helpers.
///
/// Like [`emit_errno_error_mapping`], but maps missing paths to the
/// filesystem-specific `ErrPathNotFound` instead of the generic `ErrNotFound`,
/// routes host errnos that indicate an unusable path string to `ErrInvalidPath`,
/// and (for no-follow opens) maps a final-symlink `ELOOP` to `ErrAccessDenied`.
/// The host errno is expected in `x9`, as produced by `emit_errno`.
fn emit_fs_path_errno_error_mapping(
    symbol: &str,
    target: &str,
    no_follow: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    let linux = target == "linux-aarch64";
    let eloop = if linux { "40" } else { "62" };
    let enametoolong = if linux { "36" } else { "63" };
    let eilseq = if linux { "84" } else { "92" };
    let enotempty = if linux { "39" } else { "66" };

    let err_path_not_found = format!("{symbol}_errno_path_not_found");
    let err_access_denied = format!("{symbol}_errno_access_denied");
    let err_already_exists = format!("{symbol}_errno_already_exists");
    let err_not_empty = format!("{symbol}_errno_not_empty");
    let err_invalid_path = format!("{symbol}_errno_invalid_path");
    let err_output = format!("{symbol}_errno_output");
    let eloop_target = if no_follow {
        err_access_denied.clone()
    } else {
        err_invalid_path.clone()
    };

    instructions.extend([
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_path_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&err_already_exists),
        abi::compare_immediate("x9", enotempty),
        abi::branch_eq(&err_not_empty),
        abi::compare_immediate("x9", "20"),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", enametoolong),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", eilseq),
        abi::branch_eq(&err_invalid_path),
        abi::compare_immediate("x9", eloop),
        abi::branch_eq(&eloop_target),
        abi::branch(&err_output),
        abi::label(&err_path_not_found),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_PATH_NOT_FOUND_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_PATH_NOT_FOUND_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_access_denied),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ACCESS_DENIED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ACCESS_DENIED_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_ALREADY_EXISTS_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_not_empty),
        abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_DIRECTORY_NOT_EMPTY_CODE,
        ),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_DIRECTORY_NOT_EMPTY_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::branch(done),
        abi::label(&err_invalid_path),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INVALID_PATH_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_INVALID_PATH_SYMBOL, instructions, relocations);
    instructions.extend([
        abi::branch(done),
        abi::label(&err_output),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, ERR_OUTPUT_SYMBOL, instructions, relocations);
    instructions.push(abi::branch(done));
}

/// Symbol of the shared standalone UTF-8 validation runtime helper.
const VALIDATE_UTF8_SYMBOL: &str = "_mfb_rt_validate_utf8";

/// Symbol of the shared standalone string-list sort runtime helper.
const SORT_STRING_LIST_SYMBOL: &str = "_mfb_rt_sort_string_list";

/// Symbol of the shared standalone `fs::pathJoin` runtime helper.
const FS_PATH_JOIN_SYMBOL: &str = "_mfb_rt_fs_path_join";

/// Lower the standalone `fs::pathJoin` helper. It takes a `List OF String`
/// collection pointer in `x0` and returns a `Result`-shaped value: `x0` holds
/// the tag (`RESULT_OK_TAG`/`RESULT_ERR_TAG`) and, on success, `x1` holds the
/// resulting `String` pointer (on allocation failure it returns `ErrOutOfMemory`).
/// Implementing it as a shared `bl`-reachable helper lets both root native code
/// and imported-package binary_repr lower `pathJoin` identically. Components are
/// joined with `/`, empty components are skipped, an absolute component discards
/// everything accumulated so far, and duplicate separators are avoided.
fn lower_fs_path_join_helper(platform: &dyn CodegenPlatform) -> CodeFunction {
    const SEP: &str = "47";
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const PARTS_OFFSET: usize = 8;
    const RESULT_OFFSET: usize = 16;
    let symbol = FS_PATH_JOIN_SYMBOL;
    let _ = platform;

    let length_loop = format!("{symbol}_length_loop");
    let length_done = format!("{symbol}_length_done");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let build_loop = format!("{symbol}_build_loop");
    let build_done = format!("{symbol}_build_done");
    let skip_part = format!("{symbol}_skip_part");
    let absolute = format!("{symbol}_absolute");
    let copy_part = format!("{symbol}_copy_part");
    let no_separator = format!("{symbol}_no_separator");
    let copy_loop = format!("{symbol}_copy_loop");
    let copy_done = format!("{symbol}_copy_done");
    let done = format!("{symbol}_done");

    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME_SIZE),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARTS_OFFSET),
        // Pass 1: upper-bound length = sum(component lengths) + count separators.
        abi::load_u64("x9", abi::return_register(), COLLECTION_OFFSET_COUNT),
        abi::move_immediate("x11", "Integer", "0"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::add_immediate("x13", abi::return_register(), COLLECTION_HEADER_SIZE),
        abi::label(&length_loop),
        abi::compare_registers("x12", "x9"),
        abi::branch_ge(&length_done),
        abi::load_u64("x14", "x13", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_registers("x11", "x11", "x14"),
        abi::add_immediate("x13", "x13", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&length_loop),
        abi::label(&length_done),
        abi::add_registers(abi::return_register(), "x11", "x9"),
        abi::add_immediate(abi::return_register(), abi::return_register(), 9),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ];
    let mut relocations = vec![CodeRelocation {
        from: symbol.to_string(),
        to: ARENA_ALLOC_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }];
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        // Pass 2: build the joined path.
        abi::load_u64("x16", abi::stack_pointer(), PARTS_OFFSET),
        abi::load_u64("x9", "x16", COLLECTION_OFFSET_COUNT),
        // data base = collection + header + count * entry_size
        abi::add_immediate("x14", "x16", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x5", "Integer", &entry_size),
        abi::multiply_registers("x5", "x9", "x5"),
        abi::add_registers("x14", "x14", "x5"),
        abi::add_immediate("x15", "x16", COLLECTION_HEADER_SIZE),
        abi::load_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::add_immediate("x6", "x1", 8),
        abi::move_register("x13", "x6"),
        abi::move_immediate("x12", "Integer", "0"),
        abi::label(&build_loop),
        abi::compare_registers("x12", "x9"),
        abi::branch_ge(&build_done),
        abi::load_u64("x3", "x15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::compare_immediate("x3", "0"),
        abi::branch_eq(&skip_part),
        abi::load_u64("x2", "x15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x2", "x14", "x2"),
        abi::load_u8("x4", "x2", 0),
        abi::compare_immediate("x4", SEP),
        abi::branch_eq(&absolute),
        abi::compare_registers("x13", "x6"),
        abi::branch_eq(&no_separator),
        abi::subtract_immediate("x7", "x13", 1),
        abi::load_u8("x5", "x7", 0),
        abi::compare_immediate("x5", SEP),
        abi::branch_eq(&no_separator),
        abi::move_immediate("x5", "Byte", SEP),
        abi::store_u8("x5", "x13", 0),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_part),
        abi::label(&absolute),
        abi::move_register("x13", "x6"),
        abi::label(&no_separator),
        abi::label(&copy_part),
        abi::label(&copy_loop),
        abi::compare_immediate("x3", "0"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x4", "x2", 0),
        abi::store_u8("x4", "x13", 0),
        abi::add_immediate("x2", "x2", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::subtract_immediate("x3", "x3", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::label(&skip_part),
        abi::add_immediate("x15", "x15", COLLECTION_ENTRY_SIZE),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&build_loop),
        abi::label(&build_done),
        abi::subtract_registers("x4", "x13", "x6"),
        abi::load_u64("x1", abi::stack_pointer(), RESULT_OFFSET),
        abi::store_u64("x4", "x1", 0),
        abi::move_immediate("x5", "Integer", "0"),
        abi::store_u8("x5", "x13", 0),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.fsPathJoin".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame: CodeFrame {
            stack_size: FRAME_SIZE,
            callee_saved: vec![abi::link_register().to_string()],
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}

/// Lower the standalone string-list sort helper used to give `fs::listDirectory`
/// a deterministic, stable order. It takes a `List OF String` collection pointer
/// in `x0` and sorts its entries in place by ascending byte-wise (UTF-8
/// lexicographic) order using selection sort, swapping only the fixed-size entry
/// records and leaving the data region untouched. It makes no calls.
fn lower_sort_string_list_helper() -> CodeFunction {
    let symbol = SORT_STRING_LIST_SYMBOL;
    // x0  = collection pointer (preserved for the caller)
    // x9  = entries base (collection + header)
    // x10 = count
    // x11 = data region base (entries base + count * entry size)
    // x12 = i (outer index), x13 = min index, x14 = j (inner index)
    // x15 = entry[min] address, x16 = entry[j] address
    // x1..x7 = comparison/swap scratch
    let entry_size = COLLECTION_ENTRY_SIZE.to_string();
    let done = format!("{symbol}_done");
    let outer = format!("{symbol}_outer");
    let inner = format!("{symbol}_inner");
    let inner_done = format!("{symbol}_inner_done");
    let no_swap = format!("{symbol}_no_swap");
    let next_inner = format!("{symbol}_next_inner");
    let cmp_loop = format!("{symbol}_cmp_loop");
    let take_j = format!("{symbol}_take_j");
    let keep_min = format!("{symbol}_keep_min");

    let mut instructions = vec![
        abi::label("entry"),
        abi::load_u64("x10", "x0", COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("x10", "1"),
        abi::branch_le(&done),
        abi::add_immediate("x9", "x0", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x11", "x10", "x1"),
        abi::add_registers("x11", "x9", "x11"),
        abi::move_immediate("x12", "Integer", "0"),
        // outer: for i in 0..count-1
        abi::label(&outer),
        abi::add_immediate("x2", "x12", 1),
        abi::compare_registers("x2", "x10"),
        abi::branch_ge(&done),
        abi::move_register("x13", "x12"),
        abi::move_register("x14", "x2"),
        // inner: for j in i+1..count
        abi::label(&inner),
        abi::compare_registers("x14", "x10"),
        abi::branch_ge(&inner_done),
        // entry[min] -> x15, entry[j] -> x16
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x15", "x13", "x1"),
        abi::add_registers("x15", "x9", "x15"),
        abi::multiply_registers("x16", "x14", "x1"),
        abi::add_registers("x16", "x9", "x16"),
        // name pointers: data_base + value_offset ; lengths: value_length
        abi::load_u64("x2", "x15", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x2", "x11", "x2"),
        abi::load_u64("x3", "x15", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::load_u64("x4", "x16", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::add_registers("x4", "x11", "x4"),
        abi::load_u64("x5", "x16", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        // compare bytes: x2/x3 = min name ptr/len, x4/x5 = j name ptr/len
        abi::move_immediate("x6", "Integer", "0"),
        abi::label(&cmp_loop),
        // if reached end of min name -> min is prefix; j<min iff j also ended? no: min shorter => min<j => keep_min
        abi::compare_registers("x6", "x3"),
        abi::branch_ge(&keep_min),
        // if reached end of j name -> j shorter, j<min => take_j
        abi::compare_registers("x6", "x5"),
        abi::branch_ge(&take_j),
        abi::load_u8("x7", "x2", 0),
        abi::load_u8("x1", "x4", 0),
        abi::compare_registers("x1", "x7"),
        abi::branch_lo(&take_j),
        abi::branch_hi(&keep_min),
        abi::add_immediate("x2", "x2", 1),
        abi::add_immediate("x4", "x4", 1),
        abi::add_immediate("x6", "x6", 1),
        abi::branch(&cmp_loop),
        abi::label(&take_j),
        abi::move_register("x13", "x14"),
        abi::label(&keep_min),
        abi::label(&next_inner),
        abi::add_immediate("x14", "x14", 1),
        abi::branch(&inner),
        abi::label(&inner_done),
        // swap entry[i] and entry[min] if different
        abi::compare_registers("x13", "x12"),
        abi::branch_eq(&no_swap),
        abi::move_immediate("x1", "Integer", &entry_size),
        abi::multiply_registers("x2", "x12", "x1"),
        abi::add_registers("x2", "x9", "x2"),
        abi::multiply_registers("x3", "x13", "x1"),
        abi::add_registers("x3", "x9", "x3"),
    ];
    // swap COLLECTION_ENTRY_SIZE bytes (8 at a time)
    let mut offset = 0;
    while offset < COLLECTION_ENTRY_SIZE {
        instructions.extend([
            abi::load_u64("x4", "x2", offset),
            abi::load_u64("x5", "x3", offset),
            abi::store_u64("x5", "x2", offset),
            abi::store_u64("x4", "x3", offset),
        ]);
        offset += 8;
    }
    instructions.extend([
        abi::label(&no_swap),
        abi::add_immediate("x12", "x12", 1),
        abi::branch(&outer),
        abi::label(&done),
        abi::return_(),
    ]);
    CodeFunction {
        name: "runtime.sortStringList".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Emit a call to the shared [`VALIDATE_UTF8_SYMBOL`] helper. The byte pointer
/// must already be in `x0` and the byte length in `x1`. The helper returns `0`
/// in `x0` for valid UTF-8 and `1` for invalid; this branches to `error_label`
/// when invalid. Keeping validation in a separate `bl`-reachable function (with
/// its own frame and short-range internal branches) keeps the filesystem read
/// helpers small.
fn emit_call_validate_utf8(
    symbol: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(VALIDATE_UTF8_SYMBOL));
    relocations.push(CodeRelocation {
        from: symbol.to_string(),
        to: VALIDATE_UTF8_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    instructions.extend([
        abi::compare_immediate("x0", "0"),
        abi::branch_ne(error_label),
    ]);
}

/// Lower the standalone UTF-8 validation helper. It takes a byte pointer in `x0`
/// and a byte length in `x1`, and returns `0` in `x0` when the buffer is
/// well-formed UTF-8 or `1` otherwise. It makes no calls, so it needs no stack
/// frame.
fn lower_validate_utf8_helper() -> CodeFunction {
    let symbol = VALIDATE_UTF8_SYMBOL;
    let invalid = format!("{symbol}_invalid");
    let mut instructions = vec![abi::label("entry")];
    if std::env::var("MFB_ASCII").is_ok() {
        let lp = format!("{symbol}_lp");
        let ok = format!("{symbol}_ok");
        instructions.extend([
            abi::move_register("x9", "x0"),
            abi::move_register("x10", "x1"),
            abi::label(&lp),
            abi::compare_immediate("x10", "0"),
            abi::branch_eq(&ok),
            abi::load_u8("x11", "x9", 0),
            abi::compare_immediate("x11", "127"),
            abi::branch_hi(&invalid),
            abi::add_immediate("x9", "x9", 1),
            abi::subtract_immediate("x10", "x10", 1),
            abi::branch(&lp),
            abi::label(&ok),
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    } else {
        emit_validate_utf8(symbol, "x0", "x1", &invalid, &mut instructions);
        instructions.extend([
            abi::move_immediate("x0", "Integer", "0"),
            abi::return_(),
            abi::label(&invalid),
            abi::move_immediate("x0", "Integer", "1"),
            abi::return_(),
        ]);
    }
    CodeFunction {
        name: "runtime.validateUtf8".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// Validate that the `len`-byte buffer at `ptr` is well-formed UTF-8, branching
/// to `error_label` on the first invalid sequence. Used by
/// [`lower_validate_utf8_helper`]. Clobbers `x9`-`x14`. `ptr` and `len` are read
/// into scratch registers before any clobber, so they may name `x0`/`x1`.
fn emit_validate_utf8(
    symbol: &str,
    ptr: &str,
    len: &str,
    error_label: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let pos = "x9";
    let rem = "x10";
    let byte = "x11";
    let cont = "x12";
    let lo = "x13";
    let hi = "x14";

    let loop_start = format!("{symbol}_utf8_loop");
    let done = format!("{symbol}_utf8_done");
    let one = format!("{symbol}_utf8_one");
    let two = format!("{symbol}_utf8_two");
    let three = format!("{symbol}_utf8_three");
    let four = format!("{symbol}_utf8_four");
    let three_ed = format!("{symbol}_utf8_three_ed");
    let three_bounds = format!("{symbol}_utf8_three_bounds");
    let four_f4 = format!("{symbol}_utf8_four_f4");
    let four_bounds = format!("{symbol}_utf8_four_bounds");

    instructions.extend([
        abi::move_register(pos, ptr),
        abi::move_register(rem, len),
        abi::label(&loop_start),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&done),
        abi::load_u8(byte, pos, 0),
        abi::compare_immediate(byte, "128"),
        abi::branch_lo(&one),
        abi::compare_immediate(byte, "194"),
        abi::branch_lo(error_label),
        abi::compare_immediate(byte, "224"),
        abi::branch_lo(&two),
        abi::compare_immediate(byte, "240"),
        abi::branch_lo(&three),
        abi::compare_immediate(byte, "245"),
        abi::branch_lo(&four),
        abi::branch(error_label),
        // 1-byte ASCII
        abi::label(&one),
        abi::add_immediate(pos, pos, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_start),
        // 2-byte sequence
        abi::label(&two),
        abi::compare_immediate(rem, "2"),
        abi::branch_lo(error_label),
        abi::load_u8(cont, pos, 1),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 2),
        abi::subtract_immediate(rem, rem, 2),
        abi::branch(&loop_start),
        // 3-byte sequence
        abi::label(&three),
        abi::compare_immediate(rem, "3"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "224"),
        abi::branch_ne(&three_ed),
        abi::move_immediate(lo, "Integer", "160"),
        abi::branch(&three_bounds),
        abi::label(&three_ed),
        abi::compare_immediate(byte, "237"),
        abi::branch_ne(&three_bounds),
        abi::move_immediate(hi, "Integer", "159"),
        abi::label(&three_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 3),
        abi::subtract_immediate(rem, rem, 3),
        abi::branch(&loop_start),
        // 4-byte sequence
        abi::label(&four),
        abi::compare_immediate(rem, "4"),
        abi::branch_lo(error_label),
        abi::move_immediate(lo, "Integer", "128"),
        abi::move_immediate(hi, "Integer", "191"),
        abi::compare_immediate(byte, "240"),
        abi::branch_ne(&four_f4),
        abi::move_immediate(lo, "Integer", "144"),
        abi::branch(&four_bounds),
        abi::label(&four_f4),
        abi::compare_immediate(byte, "244"),
        abi::branch_ne(&four_bounds),
        abi::move_immediate(hi, "Integer", "143"),
        abi::label(&four_bounds),
        abi::load_u8(cont, pos, 1),
        abi::compare_registers(cont, lo),
        abi::branch_lo(error_label),
        abi::compare_registers(cont, hi),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 2),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::load_u8(cont, pos, 3),
        abi::compare_immediate(cont, "128"),
        abi::branch_lo(error_label),
        abi::compare_immediate(cont, "191"),
        abi::branch_hi(error_label),
        abi::add_immediate(pos, pos, 4),
        abi::subtract_immediate(rem, rem, 4),
        abi::branch(&loop_start),
        abi::label(&done),
    ]);
}

/// Materialize the address of an internal symbol (data or code) into `dst` via
/// the `adrp`/`add` page pair. The `data` binding is the internal-symbol-address
/// relocation regardless of the target's section — the linker resolves it through
/// `symbol_vmaddr` (the same pattern used for the thread-trampoline address).
fn push_symbol_address(
    from: &str,
    symbol: &str,
    dst: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::load_page_address(dst, symbol));
    instructions.push(abi::add_page_offset(dst, dst, symbol));
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

fn push_error_message_address(
    from: &str,
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

fn finalize_frame(
    instructions: &mut Vec<CodeInstruction>,
    stack_slots: &mut [CodeStackSlot],
    local_stack_size: usize,
    mut callee_saved: Vec<String>,
) -> CodeFrame {
    if instructions.iter().any(|instruction| {
        instruction.op == CodeOp::BranchLink || instruction.op == CodeOp::BranchLinkRegister
    }) && !callee_saved
        .iter()
        .any(|register| register == abi::link_register())
    {
        callee_saved.push(abi::link_register().to_string());
    }
    let save_size = callee_saved.len() * 8;
    let total_stack_size = align(save_size + local_stack_size, 16);
    if total_stack_size == 0 {
        return CodeFrame {
            stack_size: 0,
            callee_saved,
        };
    }

    for slot in stack_slots {
        slot.offset += save_size as i32;
    }
    adjust_stack_instruction_offsets(instructions, save_size);

    let mut prologue = Vec::new();
    prologue.push(abi::subtract_stack(total_stack_size));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(abi::store_u64(register, abi::stack_pointer(), index * 8));
    }

    let insert_at = if instructions
        .first()
        .is_some_and(|instruction| instruction.op == CodeOp::Label)
    {
        1
    } else {
        0
    };
    instructions.splice(insert_at..insert_at, prologue);

    let mut rewritten = Vec::new();
    for instruction in instructions.drain(..) {
        if instruction.op == CodeOp::Ret {
            for (index, register) in callee_saved.iter().enumerate().rev() {
                rewritten.push(abi::load_u64(register, abi::stack_pointer(), index * 8));
            }
            rewritten.push(abi::add_stack(total_stack_size));
            rewritten.push(instruction);
        } else {
            rewritten.push(instruction);
        }
    }
    *instructions = rewritten;

    CodeFrame {
        stack_size: total_stack_size,
        callee_saved,
    }
}

fn adjust_stack_instruction_offsets(instructions: &mut [CodeInstruction], offset_delta: usize) {
    if offset_delta == 0 {
        return;
    }
    for instruction in instructions {
        let stack_relative = instruction
            .fields
            .iter()
            .any(|(name, value)| matches!(*name, "base" | "src") && abi::is_stack_pointer(value));
        if !stack_relative {
            continue;
        }
        for (name, value) in &mut instruction.fields {
            if matches!(*name, "offset" | "imm") {
                if let Ok(offset) = value.parse::<usize>() {
                    *value = (offset + offset_delta).to_string();
                }
            }
        }
    }
}

mod net;
mod term;
mod tls;
mod link_thunk;
mod builder_collection_layout;
mod builder_collection_queries;
mod builder_collection_updates;
mod builder_control;
mod builder_conversions;
mod builder_fixed_math;
mod builder_fs_paths;
mod builder_math;
mod builder_misc;
mod builder_numeric;
mod builder_search;
mod builder_strings;
mod builder_strings_package;
mod builder_values;
mod private;

impl CodeInstruction {
    pub(crate) fn new(op: &str) -> Self {
        Self {
            op: CodeOp::from_mnemonic(op).unwrap_or_else(|err| panic!("{err}")),
            fields: Vec::new(),
        }
    }

    pub(crate) fn field(mut self, name: &'static str, value: &str) -> Self {
        self.fields.push((name, value.to_string()));
        self
    }

    fn validate(&self) -> Result<(), String> {
        let required: &[&str] = match self.op {
            CodeOp::Label => &["name"],
            CodeOp::Mov => &["dst", "src"],
            CodeOp::MovImm => &["dst", "value"],
            CodeOp::Add
            | CodeOp::Adds
            | CodeOp::Sub
            | CodeOp::Subs
            | CodeOp::And
            | CodeOp::Orr
            | CodeOp::Eor
            | CodeOp::Mul
            | CodeOp::SMulH
            | CodeOp::UMulH
            | CodeOp::Adc
            | CodeOp::Rorv
            | CodeOp::SDiv
            | CodeOp::UDiv
            | CodeOp::FAddD
            | CodeOp::FSubD
            | CodeOp::FMulD
            | CodeOp::FDivD => &["dst", "lhs", "rhs"],
            CodeOp::Mvn => &["dst", "src"],
            CodeOp::MSub => &["dst", "lhs", "rhs", "minuend"],
            CodeOp::LslImm | CodeOp::LsrImm | CodeOp::AsrImm => &["dst", "src", "shift"],
            CodeOp::AddImm | CodeOp::SubImm => &["dst", "src", "imm"],
            CodeOp::SubSp | CodeOp::AddSp => &["imm"],
            CodeOp::CmpImm => &["lhs", "rhs"],
            CodeOp::Cmp => &["lhs", "rhs"],
            CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::Branch
            | CodeOp::BranchLink => &["target"],
            CodeOp::BranchLinkRegister => &["register"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 | CodeOp::LdrU32 | CodeOp::LdrU16 | CodeOp::LdrU8 => {
                &["dst", "base", "offset"]
            }
            CodeOp::StrU64 | CodeOp::StrU32 | CodeOp::StrU8 => &["src", "base", "offset"],
            CodeOp::Adrp | CodeOp::AddPageOff => &["dst", "symbol"],
            CodeOp::FMovXFromD
            | CodeOp::FMovDFromX
            | CodeOp::FNegD
            | CodeOp::FSqrtD
            | CodeOp::SCvtfDFromX
            | CodeOp::FCvtzsXFromD
            | CodeOp::FCvtmsXFromD
            | CodeOp::FCvtpsXFromD
            | CodeOp::FCvtasXFromD => &["dst", "src"],
            CodeOp::FCmpD => &["lhs", "rhs"],
            CodeOp::FCmpZeroD => &["src"],
        };
        for name in required {
            if !self.fields.iter().any(|(field, _)| field == name) {
                return Err(format!(
                    "native code instruction '{}' missing field '{}'",
                    self.op.mnemonic(),
                    name
                ));
            }
        }
        Ok(())
    }
}

trait ToCodeJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToCodeJson for CodeFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"frame\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"stackSlots\": [{}\n{}  ],\n",
                "{}  \"instructions\": [{}\n{}  ],\n",
                "{}  \"relocations\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.returns),
            pad,
            self.frame.to_json(indent + 2),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.stack_slots, indent + 2),
            pad,
            pad,
            join_json(&self.instructions, indent + 2),
            pad,
            pad,
            join_json(&self.relocations, indent + 2),
            pad,
            pad
        )
    }
}

impl CodeFrame {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"stackSize\": {}, \"calleeSaved\": [{}] }}",
            self.stack_size,
            json_string_list(&self.callee_saved)
        )
    }
}

impl ToCodeJson for CodeParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"location\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            json_string(&self.location)
        )
    }
}

impl ToCodeJson for CodeInstruction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut fields = vec![format!("\"op\": {}", json_string(self.op.mnemonic()))];
        fields.extend(
            self.fields
                .iter()
                .map(|(name, value)| format!("\"{name}\": {}", json_string(value))),
        );
        format!("\n{}{{ {} }}", pad, fields.join(", "))
    }
}

impl ToCodeJson for CodeRelocation {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let library = self
            .library
            .as_ref()
            .map(|library| json_string(library))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"kind\": {}, \"binding\": {}, \"library\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(&self.kind),
            json_string(&self.binding),
            library
        )
    }
}

impl ToCodeJson for CodeImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol)
        )
    }
}

impl ToCodeJson for CodeDataObject {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{ \"symbol\": {}, \"kind\": {}, \"layout\": {}, ",
                "\"align\": {}, \"size\": {}, \"value\": {} }}"
            ),
            pad,
            json_string(&self.symbol),
            json_string(&self.kind),
            json_string(&self.layout),
            self.align,
            self.size,
            json_string(&self.value)
        )
    }
}

impl ToCodeJson for CodeStackSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"offset\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            self.offset
        )
    }
}

fn string_symbols(module: &NirModule) -> HashMap<String, String> {
    let mut values = Vec::new();
    if module_uses_type_name(module) {
        collect_type_name_values(module, &mut values);
    }
    for function in &module.functions {
        collect_string_values_from_ops(&function.body, &mut values);
    }
    // Source file paths back `ErrorLoc.filename` for errors that originate in
    // each function; emit them as string constants so the origin can load them.
    for function in &module.functions {
        if !function.file.is_empty() {
            push_string_value(&mut values, function.file.clone());
        }
    }
    for value in [
        ERR_INVALID_ARGUMENT_MESSAGE,
        ERR_OVERFLOW_MESSAGE,
        ERR_UNDERFLOW_MESSAGE,
        ERR_ALLOCATION_MESSAGE,
    ] {
        push_string_value(&mut values, value.to_string());
    }
    if module_may_emit_float_numeric_error(module) {
        for value in [
            ERR_FLOAT_DOMAIN_MESSAGE,
            ERR_FLOAT_NAN_MESSAGE,
            ERR_FLOAT_INF_MESSAGE,
            ERR_FLOAT_OVERFLOW_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_any_call(
        module,
        &[
            "io.print",
            "io.write",
            "io.printError",
            "io.writeError",
            "io.flush",
            "io.flushError",
        ],
    ) {
        push_string_value(&mut values, ERR_OUTPUT_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &["io.input", "io.readLine", "io.readChar", "io.readByte"],
    ) {
        if module_uses_call(module, "io.input") {
            push_string_value(&mut values, String::new());
        }
        push_string_value(&mut values, ERR_EOF_MESSAGE.to_string());
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
    }
    if module_uses_call(module, "io.pollInput") {
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &[
            "thread.isRunning",
            "thread.waitFor",
            "thread.cancel",
            "thread.send",
            "thread.poll",
            "thread.receive",
            "thread.read",
        ],
    ) {
        push_string_value(&mut values, ERR_RESOURCE_CLOSED_MESSAGE.to_string());
    }
    if module_uses_call(module, "fs.currentDirectory") {
        push_string_value(&mut values, ERR_READ_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &[
            "fs.setCurrentDirectory",
            "fs.deleteFile",
            "fs.createDirectory",
            "fs.deleteDirectory",
            "fs.listDirectory",
        ],
    ) {
        for value in [
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_DIRECTORY_NOT_EMPTY_MESSAGE,
            ERR_OUTPUT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_any_call(
        module,
        &[
            "fs.open",
            "fs.openFile",
            "fs.openFileNoFollow",
            "fs.canonicalPath",
            "fs.isWithin",
            "fs.writeTextAtomic",
            "fs.writeBytesAtomic",
            "fs.close",
            "fs.writeAll",
        ],
    ) {
        for value in [
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_OUTPUT_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_call(module, "term.terminalSize") {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &[
            "net.lookup",
            "net.connectTcp",
            "net.listenTcp",
            "net.accept",
            "net.poll",
            "net.read",
            "net.readText",
            "net.write",
            "net.writeText",
            "net.close",
            "net.localAddress",
            "net.remoteAddress",
            "net.setReadTimeout",
            "net.setWriteTimeout",
            "net.bindUdp",
            "net.receiveFrom",
            "net.receiveTextFrom",
            "net.sendTo",
            "net.sendTextTo",
        ],
    ) {
        for value in [
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_READ_TIMEOUT_MESSAGE,
            ERR_WRITE_TIMEOUT_MESSAGE,
            ERR_MESSAGE_TOO_LARGE_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_CLOSE_FAILED_MESSAGE,
            ERR_ENCODING_MESSAGE,
            ERR_TIMEOUT_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_any_call(
        module,
        &[
            "tls.connect",
            "tls.read",
            "tls.readText",
            "tls.write",
            "tls.writeText",
            "tls.close",
        ],
    ) {
        for value in [
            ERR_TLS_FAILED_MESSAGE,
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_ENCODING_MESSAGE,
        ] {
            push_string_value(&mut values, value.to_string());
        }
    }
    if module_uses_migrated(module, "find")
        || module_uses_migrated(module, "mid")
        || module_uses_migrated(module, "get")
        || module_uses_migrated(module, "append")
        || module_uses_migrated(module, "prepend")
        || module_uses_migrated(module, "insert")
        || module_uses_migrated(module, "transform")
        || module_uses_migrated(module, "filter")
        || module_uses_migrated(module, "removeAt")
        || module_uses_migrated(module, "set")
        || module_uses_call(module, "strings.graphemeAt")
    {
        push_string_value(&mut values, ERR_INDEX_OUT_OF_RANGE_MESSAGE.to_string());
    }
    if module_uses_migrated(module, "find") || module_uses_migrated(module, "get") {
        push_string_value(&mut values, ERR_NOT_FOUND_MESSAGE.to_string());
    }
    if module_uses_call(module, "toString") {
        push_string_value(&mut values, "TRUE".to_string());
        push_string_value(&mut values, "FALSE".to_string());
        push_string_value(&mut values, FLOAT_TO_STRING_FORMAT.to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
    }
    for value in [
        ENTRY_ERROR_PREFIX,
        ENTRY_ERROR_SEPARATOR,
        ENTRY_ERROR_NEWLINE,
    ] {
        if !values.contains(&value.to_string()) {
            values.push(value.to_string());
        }
    }
    if module_may_record_cleanup_failure(module) {
        for value in [CLEANUP_FAILURE_PREFIX, CLEANUP_FAILURE_SEPARATOR] {
            if !values.contains(&value.to_string()) {
                values.push(value.to_string());
            }
        }
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let symbol = if let Some(symbol) = standard_error_message_symbol(&value) {
                symbol.to_string()
            } else if value == ENTRY_ERROR_PREFIX {
                ENTRY_ERROR_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_SEPARATOR {
                ENTRY_ERROR_SEPARATOR_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_NEWLINE {
                ENTRY_ERROR_NEWLINE_SYMBOL.to_string()
            } else if value == CLEANUP_FAILURE_PREFIX {
                CLEANUP_FAILURE_PREFIX_SYMBOL.to_string()
            } else if value == CLEANUP_FAILURE_SEPARATOR {
                CLEANUP_FAILURE_SEPARATOR_SYMBOL.to_string()
            } else {
                format!("_mfb_str_{index}")
            };
            (value, symbol)
        })
        .collect()
}

/// Error messages emitted by native `LINK` thunks and their initializer
/// (plan-linker.md §12): the allocation message is already covered by the
/// standard set, so only the two binding-specific messages are listed here.
fn native_link_error_messages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            ERR_NATIVE_LINK_LOAD_CODE,
            ERR_NATIVE_LINK_LOAD_MESSAGE,
            ERR_NATIVE_LINK_LOAD_SYMBOL,
        ),
        (
            ERR_NATIVE_LINK_CALL_CODE,
            ERR_NATIVE_LINK_CALL_MESSAGE,
            ERR_NATIVE_LINK_CALL_SYMBOL,
        ),
        (
            ERR_OUT_OF_MEMORY_CODE,
            ERR_ALLOCATION_MESSAGE,
            ERR_ALLOCATION_SYMBOL,
        ),
        // Boundary validations (plan-linker.md §12.3/§12.4).
        (ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE, ERR_OVERFLOW_SYMBOL),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
        (ERR_FLOAT_NAN_CODE, ERR_FLOAT_NAN_MESSAGE, ERR_FLOAT_NAN_SYMBOL),
        (ERR_FLOAT_INF_CODE, ERR_FLOAT_INF_MESSAGE, ERR_FLOAT_INF_SYMBOL),
    ]
}

fn standard_error_messages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            ERR_INVALID_ARGUMENT_CODE,
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_INVALID_ARGUMENT_SYMBOL,
        ),
        (
            ERR_INVALID_FORMAT_CODE,
            ERR_INVALID_FORMAT_MESSAGE,
            ERR_INVALID_FORMAT_SYMBOL,
        ),
        (ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE, ERR_OVERFLOW_SYMBOL),
        (
            ERR_UNDERFLOW_CODE,
            ERR_UNDERFLOW_MESSAGE,
            ERR_UNDERFLOW_SYMBOL,
        ),
        (
            ERR_FLOAT_DOMAIN_CODE,
            ERR_FLOAT_DOMAIN_MESSAGE,
            ERR_FLOAT_DOMAIN_SYMBOL,
        ),
        (
            ERR_FLOAT_NAN_CODE,
            ERR_FLOAT_NAN_MESSAGE,
            ERR_FLOAT_NAN_SYMBOL,
        ),
        (
            ERR_FLOAT_INF_CODE,
            ERR_FLOAT_INF_MESSAGE,
            ERR_FLOAT_INF_SYMBOL,
        ),
        (
            ERR_FLOAT_OVERFLOW_CODE,
            ERR_FLOAT_OVERFLOW_MESSAGE,
            ERR_FLOAT_OVERFLOW_SYMBOL,
        ),
        (
            ERR_OUT_OF_MEMORY_CODE,
            ERR_ALLOCATION_MESSAGE,
            ERR_ALLOCATION_SYMBOL,
        ),
        (
            ERR_INDEX_OUT_OF_RANGE_CODE,
            ERR_INDEX_OUT_OF_RANGE_MESSAGE,
            ERR_INDEX_OUT_OF_RANGE_SYMBOL,
        ),
        (
            ERR_NOT_FOUND_CODE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_NOT_FOUND_SYMBOL,
        ),
        (ERR_TIMEOUT_CODE, ERR_TIMEOUT_MESSAGE, ERR_TIMEOUT_SYMBOL),
        (
            ERR_INTERRUPTED_CODE,
            ERR_INTERRUPTED_MESSAGE,
            ERR_INTERRUPTED_SYMBOL,
        ),
        (ERR_READ_CODE, ERR_READ_MESSAGE, ERR_READ_SYMBOL),
        (
            ERR_ALREADY_EXISTS_CODE,
            ERR_ALREADY_EXISTS_MESSAGE,
            ERR_ALREADY_EXISTS_SYMBOL,
        ),
        (
            ERR_ACCESS_DENIED_CODE,
            ERR_ACCESS_DENIED_MESSAGE,
            ERR_ACCESS_DENIED_SYMBOL,
        ),
        (
            ERR_DIRECTORY_NOT_EMPTY_CODE,
            ERR_DIRECTORY_NOT_EMPTY_MESSAGE,
            ERR_DIRECTORY_NOT_EMPTY_SYMBOL,
        ),
        (
            ERR_CLOSE_FAILED_CODE,
            ERR_CLOSE_FAILED_MESSAGE,
            ERR_CLOSE_FAILED_SYMBOL,
        ),
        (
            ERR_PATH_NOT_FOUND_CODE,
            ERR_PATH_NOT_FOUND_MESSAGE,
            ERR_PATH_NOT_FOUND_SYMBOL,
        ),
        (
            ERR_INVALID_PATH_CODE,
            ERR_INVALID_PATH_MESSAGE,
            ERR_INVALID_PATH_SYMBOL,
        ),
        (
            ERR_RESOURCE_CLOSED_CODE,
            ERR_RESOURCE_CLOSED_MESSAGE,
            ERR_RESOURCE_CLOSED_SYMBOL,
        ),
        (
            ERR_UNSUPPORTED_CODE,
            ERR_UNSUPPORTED_MESSAGE,
            ERR_UNSUPPORTED_SYMBOL,
        ),
        (ERR_OUTPUT_CODE, ERR_OUTPUT_MESSAGE, ERR_OUTPUT_SYMBOL),
        (ERR_EOF_CODE, ERR_EOF_MESSAGE, ERR_EOF_SYMBOL),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
        (ERR_INPUT_CODE, ERR_INPUT_MESSAGE, ERR_INPUT_SYMBOL),
        (
            ERR_ADDRESS_INVALID_CODE,
            ERR_ADDRESS_INVALID_MESSAGE,
            ERR_ADDRESS_INVALID_SYMBOL,
        ),
        (
            ERR_ADDRESS_NOT_FOUND_CODE,
            ERR_ADDRESS_NOT_FOUND_MESSAGE,
            ERR_ADDRESS_NOT_FOUND_SYMBOL,
        ),
        (
            ERR_NETWORK_FAILED_CODE,
            ERR_NETWORK_FAILED_MESSAGE,
            ERR_NETWORK_FAILED_SYMBOL,
        ),
        (
            ERR_CONNECTION_CLOSED_CODE,
            ERR_CONNECTION_CLOSED_MESSAGE,
            ERR_CONNECTION_CLOSED_SYMBOL,
        ),
        (
            ERR_READ_TIMEOUT_CODE,
            ERR_READ_TIMEOUT_MESSAGE,
            ERR_READ_TIMEOUT_SYMBOL,
        ),
        (
            ERR_WRITE_TIMEOUT_CODE,
            ERR_WRITE_TIMEOUT_MESSAGE,
            ERR_WRITE_TIMEOUT_SYMBOL,
        ),
        (
            ERR_MESSAGE_TOO_LARGE_CODE,
            ERR_MESSAGE_TOO_LARGE_MESSAGE,
            ERR_MESSAGE_TOO_LARGE_SYMBOL,
        ),
        (
            ERR_TLS_FAILED_CODE,
            ERR_TLS_FAILED_MESSAGE,
            ERR_TLS_FAILED_SYMBOL,
        ),
    ]
}

fn standard_error_message_symbol(message: &str) -> Option<&'static str> {
    standard_error_messages()
        .iter()
        .find_map(|(_, candidate, symbol)| (*candidate == message).then_some(*symbol))
}

fn module_uses_type_name(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_type_name(&function.body))
}

fn module_uses_call(module: &NirModule, target: &str) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_call(&function.body, target))
        || module_drops_resource_union_close(module, target)
}

/// Whether the module uses a migrated `collections::`/`strings::` member whose
/// bare native lowering name is `bare` (e.g. `bare = "find"` checks both
/// `collections.find` and `strings.find`). The native ops keep their bare
/// lowering but arrive with the qualified target (plan-01-functions.md §5).
fn module_uses_migrated(module: &NirModule, bare: &str) -> bool {
    module_uses_call(module, &format!("collections.{bare}"))
        || module_uses_call(module, &format!("strings.{bare}"))
}

/// Whether the module binds a resource union whose tag-dispatched drop calls
/// `target` (a variant's close op). These calls are codegen-emitted rather than
/// NIR calls, so they must still pull in the close helper.
fn module_drops_resource_union_close(module: &NirModule, target: &str) -> bool {
    let unions: std::collections::HashSet<&str> = module
        .types
        .iter()
        .filter(|type_| {
            type_.kind == "union"
                && !type_.variants.is_empty()
                && type_
                    .variants
                    .iter()
                    .all(|variant| crate::builtins::is_resource_type(&variant.name))
                && type_
                    .variants
                    .iter()
                    .any(|variant| crate::builtins::resource_close_function(&variant.name) == Some(target))
        })
        .map(|type_| type_.name.as_str())
        .collect();
    if unions.is_empty() {
        return false;
    }
    module
        .functions
        .iter()
        .any(|function| ops_bind_type_in(&function.body, &unions))
}

fn ops_bind_type_in(ops: &[NirOp], names: &std::collections::HashSet<&str>) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } => names.contains(type_.as_str()),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_bind_type_in(then_body, names) || ops_bind_type_in(else_body, names),
        NirOp::Match { cases, .. } => {
            cases.iter().any(|case| ops_bind_type_in(&case.body, names))
        }
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_bind_type_in(body, names),
        _ => false,
    })
}

fn module_may_record_cleanup_failure(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_may_record_cleanup_failure(&function.body))
}

fn ops_may_record_cleanup_failure(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } => crate::builtins::resource_close_function(type_).is_some(),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_may_record_cleanup_failure(then_body) || ops_may_record_cleanup_failure(else_body),
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| ops_may_record_cleanup_failure(&case.body)),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_may_record_cleanup_failure(body),
        NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    })
}

fn module_uses_any_call(module: &NirModule, targets: &[&str]) -> bool {
    targets
        .iter()
        .any(|target| module_uses_call(module, target))
}

fn module_may_emit_float_numeric_error(module: &NirModule) -> bool {
    if module_uses_any_call(
        module,
        &[
            "math.sqrt",
            "math.pow",
            "math.atan2",
            "math.exp",
            "math.log",
            "math.log10",
            "math.sin",
            "math.cos",
            "math.tan",
            "math.asin",
            "math.acos",
            "math.atan",
        ],
    ) {
        return true;
    }
    if module.globals.iter().any(|global| {
        global
            .value
            .as_ref()
            .is_some_and(|value| value_may_emit_float_arithmetic_error(value, &HashMap::new()))
    }) {
        return true;
    }
    module.functions.iter().any(|function| {
        let mut locals = function
            .params
            .iter()
            .map(|param| (param.name.clone(), param.type_.clone()))
            .collect::<HashMap<_, _>>();
        ops_may_emit_float_arithmetic_error(&function.body, &mut locals)
    })
}

fn ops_may_emit_float_arithmetic_error(
    ops: &[NirOp],
    locals: &mut HashMap<String, String>,
) -> bool {
    for op in ops {
        let emits = match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                let emits = value
                    .as_ref()
                    .is_some_and(|value| value_may_emit_float_arithmetic_error(value, locals));
                if !type_.is_empty() {
                    locals.insert(name.clone(), type_.clone());
                }
                emits
            }
            NirOp::StoreGlobal { value, .. } | NirOp::Return { value } => value
                .as_ref()
                .is_some_and(|value| value_may_emit_float_arithmetic_error(value, locals)),
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
            NirOp::ExitProgram { code } => value_may_emit_float_arithmetic_error(code, locals),
            NirOp::Fail { error } => value_may_emit_float_arithmetic_error(error, locals),
            NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => {
                value_may_emit_float_arithmetic_error(value, locals)
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                value_may_emit_float_arithmetic_error(condition, locals)
                    || ops_may_emit_float_arithmetic_error(then_body, &mut locals.clone())
                    || ops_may_emit_float_arithmetic_error(else_body, &mut locals.clone())
            }
            NirOp::Match { value, cases } => {
                value_may_emit_float_arithmetic_error(value, locals)
                    || cases.iter().any(|case| {
                        matches!(
                            &case.pattern,
                            NirMatchPattern::Value(value)
                                if value_may_emit_float_arithmetic_error(value, locals)
                        ) || ops_may_emit_float_arithmetic_error(&case.body, &mut locals.clone())
                    })
            }
            NirOp::While {
                condition, body, ..
            } => {
                value_may_emit_float_arithmetic_error(condition, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                let mut body_locals = locals.clone();
                body_locals.insert(name.clone(), type_.clone());
                type_ == "Float"
                    || value_may_emit_float_arithmetic_error(start, locals)
                    || value_may_emit_float_arithmetic_error(end, locals)
                    || value_may_emit_float_arithmetic_error(step, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals)
            }
            NirOp::DoUntil { body, condition } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
                    || value_may_emit_float_arithmetic_error(condition, locals)
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                let mut body_locals = locals.clone();
                body_locals.insert(name.clone(), type_.clone());
                value_may_emit_float_arithmetic_error(iterable, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals)
            }
            NirOp::Trap { body, .. } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
            }
        };
        if emits {
            return true;
        }
    }
    false
}

fn value_may_emit_float_arithmetic_error(
    value: &NirValue,
    locals: &HashMap<String, String>,
) -> bool {
    match value {
        NirValue::Binary { op, left, right, .. } => {
            let result_type = static_nir_value_type(left, locals)
                .zip(static_nir_value_type(right, locals))
                .map(|(left_type, right_type)| {
                    numeric_binary_result_type(op, &left_type, &right_type)
                });
            (matches!(op.as_str(), "+" | "-" | "*" | "/" | "DIV" | "MOD" | "^")
                && result_type == Some("Float"))
                || value_may_emit_float_arithmetic_error(left, locals)
                || value_may_emit_float_arithmetic_error(right, locals)
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_emit_float_arithmetic_error(arg, locals)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => value_may_emit_float_arithmetic_error(value, locals),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_emit_float_arithmetic_error(target, locals)
                || updates
                    .iter()
                    .any(|update| value_may_emit_float_arithmetic_error(&update.value, locals))
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_emit_float_arithmetic_error(key, locals)
                || value_may_emit_float_arithmetic_error(value, locals)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_emit_float_arithmetic_error(target, locals)
        }
        NirValue::Unary { op, operand, .. } => {
            (op == "-" && static_nir_value_type(operand, locals).as_deref() == Some("Float"))
                || value_may_emit_float_arithmetic_error(operand, locals)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals)),
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => false,
    }
}

fn static_nir_value_type(value: &NirValue, locals: &HashMap<String, String>) -> Option<String> {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::Global { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::UnionExtract { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => locals.get(name).cloned(),
        NirValue::Binary { op, left, right, .. } => static_nir_value_type(left, locals)
            .zip(static_nir_value_type(right, locals))
            .map(|(left_type, right_type)| {
                numeric_binary_result_type(op, &left_type, &right_type).to_string()
            }),
        NirValue::Unary { operand, .. } => static_nir_value_type(operand, locals),
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            let arg_types = args
                .iter()
                .map(|arg| static_nir_value_type(arg, locals))
                .collect::<Option<Vec<_>>>()?;
            builtins::general::resolve_call(target, &arg_types)
                .map(|call| call.return_type.into_owned())
                .or_else(|| {
                    builtins::collections::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| {
                    builtins::strings::resolve_call(target, &arg_types)
                        .map(|call| call.return_type.into_owned())
                })
                .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
        }
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_nir_value_type(value, locals)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::MemberAccess { target, member } => {
            let target_type = static_nir_value_type(target, locals)?;
            if member == "result" {
                builtins::thread::parent_thread_output(&target_type)
                    .map(|output_type| format!("Result OF {output_type}"))
            } else {
                None
            }
        }
        NirValue::RuntimeCall { .. } | NirValue::UnionWrap { .. } | NirValue::Closure { .. } => {
            None
        }
    }
}

fn ops_use_call(ops: &[NirOp], target: &str) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. }
        | NirOp::StoreGlobal { value, .. }
        | NirOp::Return { value } => {
            value.as_ref().is_some_and(|value| value_uses_call(value, target))
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
        NirOp::ExitProgram { code } => value_uses_call(code, target),
        NirOp::Fail { error } => value_uses_call(error, target),
        NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => value_uses_call(value, target),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_call(condition, target)
                || ops_use_call(then_body, target)
                || ops_use_call(else_body, target)
        }
        NirOp::Match { value, cases } => {
            value_uses_call(value, target)
                || cases.iter().any(|case| {
                    matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_call(value, target))
                        || ops_use_call(&case.body, target)
                })
        }
        NirOp::While { condition, body, .. } => {
            value_uses_call(condition, target) || ops_use_call(body, target)
        }
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_call(start, target)
                || value_uses_call(end, target)
                || value_uses_call(step, target)
                || ops_use_call(body, target)
        }
        NirOp::DoUntil { body, condition } => {
            ops_use_call(body, target) || value_uses_call(condition, target)
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_call(iterable, target) || ops_use_call(body, target)
        }
        NirOp::Trap { body, .. } => ops_use_call(body, target),
    })
}

fn value_uses_call(value: &NirValue, target: &str) -> bool {
    match value {
        NirValue::Call {
            target: call, args, ..
        }
        | NirValue::CallResult {
            target: call, args, ..
        }
        | NirValue::RuntimeCall {
            target: call, args, ..
        } => call == target || args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::Constructor { args, .. } => args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => value_uses_call(value, target),
        NirValue::WithUpdate {
            target: updated,
            updates,
            ..
        } => {
            value_uses_call(updated, target)
                || updates
                    .iter()
                    .any(|update| value_uses_call(&update.value, target))
        }
        NirValue::ListLiteral { values, .. } => {
            values.iter().any(|value| value_uses_call(value, target))
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| value_uses_call(key, target) || value_uses_call(value, target)),
        NirValue::MemberAccess { target: value, .. } => value_uses_call(value, target),
        NirValue::Binary { left, right, .. } => {
            value_uses_call(left, target) || value_uses_call(right, target)
        }
        NirValue::Unary { operand, .. } => value_uses_call(operand, target),
        NirValue::Closure { captures, .. } => {
            captures.iter().any(|value| value_uses_call(value, target))
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}

fn ops_use_type_name(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } | NirOp::Return { value } => {
            value.as_ref().is_some_and(value_uses_type_name)
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
        NirOp::ExitProgram { code } => value_uses_type_name(code),
        NirOp::Fail { error } => value_uses_type_name(error),
        NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => value_uses_type_name(value),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_type_name(condition)
                || ops_use_type_name(then_body)
                || ops_use_type_name(else_body)
        }
        NirOp::Match { value, cases } => value_uses_type_name(value) || cases.iter().any(|case| {
            matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_type_name(value))
                || ops_use_type_name(&case.body)
        }),
        NirOp::While {
            condition, body, ..
        } => value_uses_type_name(condition) || ops_use_type_name(body),
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_type_name(start)
                || value_uses_type_name(end)
                || value_uses_type_name(step)
                || ops_use_type_name(body)
        }
        NirOp::DoUntil { body, condition } => {
            ops_use_type_name(body) || value_uses_type_name(condition)
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_type_name(iterable) || ops_use_type_name(body)
        }
        NirOp::Trap { body, .. } => ops_use_type_name(body),
    })
}

fn value_uses_type_name(value: &NirValue) -> bool {
    let direct = match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => target == "typeName",
        _ => false,
    };
    direct
        || match value {
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::RuntimeCall { args, .. }
            | NirValue::Constructor { args, .. } => args.iter().any(value_uses_type_name),
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                let _ = (union_type, member_type);
                value_uses_type_name(value)
            }
            NirValue::UnionExtract { type_, value } => {
                let _ = type_;
                value_uses_type_name(value)
            }
            NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value } => value_uses_type_name(value),
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                value_uses_type_name(target)
                    || updates
                        .iter()
                        .any(|update| value_uses_type_name(&update.value))
            }
            NirValue::ListLiteral { values, .. } => values.iter().any(value_uses_type_name),
            NirValue::MapLiteral { entries, .. } => entries
                .iter()
                .any(|(key, value)| value_uses_type_name(key) || value_uses_type_name(value)),
            NirValue::MemberAccess { target, .. } => value_uses_type_name(target),
            NirValue::Binary { left, right, .. } => {
                value_uses_type_name(left) || value_uses_type_name(right)
            }
            NirValue::Unary { operand, .. } => value_uses_type_name(operand),
            NirValue::Closure { captures, .. } => captures.iter().any(value_uses_type_name),
            NirValue::Capture { .. }
            | NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::Global { .. }
            | NirValue::FunctionRef { .. } => false,
        }
}

fn collect_type_name_values(module: &NirModule, values: &mut Vec<String>) {
    for value in [
        "Boolean", "Byte", "Error", "Fixed", "Float", "Integer", "Nothing", "String",
    ] {
        push_string_value(values, value.to_string());
    }
    for type_ in &module.types {
        push_string_value(values, type_.name.clone());
        for field in &type_.fields {
            push_string_value(values, field.type_.clone());
        }
        for variant in &type_.variants {
            push_string_value(values, variant.name.clone());
            for field in &variant.fields {
                push_string_value(values, field.type_.clone());
            }
        }
    }
    for function in &module.functions {
        push_string_value(values, function.returns.clone());
        for param in &function.params {
            push_string_value(values, param.type_.clone());
        }
        collect_type_name_values_from_ops(&function.body, values);
    }
}

fn collect_type_name_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    for op in ops {
        match op {
            NirOp::Bind { type_, value, .. } => {
                push_string_value(values, type_.clone());
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::StoreGlobal { type_, value, .. } => {
                if !type_.is_empty() {
                    push_string_value(values, type_.clone());
                }
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => collect_type_name_values_from_value(code, values),
            NirOp::Fail { error } => collect_type_name_values_from_value(error, values),
            NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => {
                collect_type_name_values_from_value(value, values);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(then_body, values);
                collect_type_name_values_from_ops(else_body, values);
            }
            NirOp::Match { value, cases } => {
                collect_type_name_values_from_value(value, values);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_type_name_values_from_value(value, values);
                    }
                    collect_type_name_values_from_ops(&case.body, values);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::For {
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(start, values);
                collect_type_name_values_from_value(end, values);
                collect_type_name_values_from_value(step, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::DoUntil { body, condition } => {
                collect_type_name_values_from_ops(body, values);
                collect_type_name_values_from_value(condition, values);
            }
            NirOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(iterable, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::Trap { body, .. } => {
                collect_type_name_values_from_ops(body, values);
            }
        }
    }
}

fn collect_type_name_values_from_value(value: &NirValue, values: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => {
            push_string_value(values, type_.clone());
        }
        NirValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            push_string_value(values, union_type.clone());
            push_string_value(values, member_type.clone());
            collect_type_name_values_from_value(value, values);
        }
        NirValue::UnionExtract { type_, value } => {
            push_string_value(values, type_.clone());
            collect_type_name_values_from_value(value, values);
        }
        _ => {}
    }
    match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_type_name_values_from_value(arg, values);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => collect_type_name_values_from_value(value, values),
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            push_string_value(values, type_.clone());
            collect_type_name_values_from_value(target, values);
            for update in updates {
                collect_type_name_values_from_value(&update.value, values);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_type_name_values_from_value(item, values);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_type_name_values_from_value(key, values);
                collect_type_name_values_from_value(value, values);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_type_name_values_from_value(target, values)
        }
        NirValue::Binary { left, right, .. } => {
            collect_type_name_values_from_value(left, values);
            collect_type_name_values_from_value(right, values);
        }
        NirValue::Unary { operand, .. } => collect_type_name_values_from_value(operand, values),
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_type_name_values_from_value(value, values);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

fn module_uses_unicode_runtime_tables(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        let mut constants = HashMap::new();
        let mut types = HashMap::new();
        ops_use_unicode_runtime_tables(&function.body, &mut constants, &mut types)
    })
}

fn ops_use_unicode_runtime_tables(
    ops: &[NirOp],
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) -> bool {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    if value_uses_unicode_runtime_tables(value, constants, types) {
                        return true;
                    }
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StateAssign { value, .. } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
            }
            NirOp::Assign { name, value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if value
                    .as_ref()
                    .is_some_and(|value| value_uses_unicode_runtime_tables(value, constants, types))
                {
                    return true;
                }
            }
            NirOp::Eval { value } | NirOp::Fail { error: value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
            }
            NirOp::Return { value } => {
                if value
                    .as_ref()
                    .is_some_and(|value| value_uses_unicode_runtime_tables(value, constants, types))
                {
                    return true;
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                if value_uses_unicode_runtime_tables(code, constants, types) {
                    return true;
                }
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut then_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_constants = constants.clone();
                let mut else_types = types.clone();
                if ops_use_unicode_runtime_tables(then_body, &mut then_constants, &mut then_types)
                    || ops_use_unicode_runtime_tables(
                        else_body,
                        &mut else_constants,
                        &mut else_types,
                    )
                {
                    return true;
                }
            }
            NirOp::Match { value, cases } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        if value_uses_unicode_runtime_tables(value, constants, types) {
                            return true;
                        }
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    if ops_use_unicode_runtime_tables(
                        &case.body,
                        &mut case_constants,
                        &mut case_types,
                    ) {
                        return true;
                    }
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                if value_uses_unicode_runtime_tables(start, constants, types)
                    || value_uses_unicode_runtime_tables(end, constants, types)
                    || value_uses_unicode_runtime_tables(step, constants, types)
                {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types)
                    || value_uses_unicode_runtime_tables(condition, constants, types)
                {
                    return true;
                }
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                if value_uses_unicode_runtime_tables(iterable, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut trap_constants, &mut trap_types) {
                    return true;
                }
            }
        }
    }
    false
}

fn value_uses_unicode_runtime_tables(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            matches!(
                target.as_str(),
                "strings.upper"
                    | "strings.lower"
                    | "strings.caseFold"
                    | "strings.normalizeNfc"
                    | "strings.graphemes"
                    | "strings.graphemeAt"
                    | "strings.graphemesCount"
            ) && !unicode_string_call_is_static(target, args, constants, types)
                || args
                    .iter()
                    .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types))
        }
        NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            value_uses_unicode_runtime_tables(value, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_unicode_runtime_tables(target, constants, types)
                || updates.iter().any(|update| {
                    value_uses_unicode_runtime_tables(&update.value, constants, types)
                })
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_uses_unicode_runtime_tables(key, constants, types)
                || value_uses_unicode_runtime_tables(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_uses_unicode_runtime_tables(target, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            value_uses_unicode_runtime_tables(left, constants, types)
                || value_uses_unicode_runtime_tables(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_uses_unicode_runtime_tables(operand, constants, types)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types)),
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}

fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types).is_some()
}

fn unicode_runtime_data_objects() -> Vec<CodeDataObject> {
    let tables = crate::unicode_runtime_tables::tables();
    vec![
        raw_data_object(
            UNICODE_STAGE1_SYMBOL,
            "u16 utf8proc stage1 property index table",
            tables.stage1.len() * 2,
            crate::unicode_runtime_tables::stage1_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_STAGE2_SYMBOL,
            "u16 utf8proc stage2 property index table",
            tables.stage2.len() * 2,
            crate::unicode_runtime_tables::stage2_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_PROPERTIES_SYMBOL,
            "mfb.unicode.property.v1 records, 24 bytes each",
            tables.properties.len() * 24,
            crate::unicode_runtime_tables::properties_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_SEQUENCES_SYMBOL,
            "u16 utf8proc sequence table",
            tables.sequences.len() * 2,
            crate::unicode_runtime_tables::sequences_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            "u32 utf8proc composition second codepoint table",
            tables.combinations_second.len() * 4,
            crate::unicode_runtime_tables::combinations_second_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_COMBINED_SYMBOL,
            "u32 utf8proc composition combined codepoint table",
            tables.combinations_combined.len() * 4,
            crate::unicode_runtime_tables::combinations_combined_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_ENTRIES_SYMBOL,
            "mfb.unicode.nfd_entry.v1 records, 16 bytes each",
            tables.nfd_entries.len() * 16,
            crate::unicode_runtime_tables::nfd_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode NFD sequence table",
            tables.nfd_sequences.len() * 4,
            crate::unicode_runtime_tables::nfd_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 uppercase records, 16 bytes each",
            tables.uppercase_entries.len() * 16,
            crate::unicode_runtime_tables::uppercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode uppercase sequence table",
            tables.uppercase_sequences.len() * 4,
            crate::unicode_runtime_tables::uppercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 lowercase records, 16 bytes each",
            tables.lowercase_entries.len() * 16,
            crate::unicode_runtime_tables::lowercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode lowercase sequence table",
            tables.lowercase_sequences.len() * 4,
            crate::unicode_runtime_tables::lowercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 casefold records, 16 bytes each",
            tables.casefold_entries.len() * 16,
            crate::unicode_runtime_tables::casefold_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode casefold sequence table",
            tables.casefold_sequences.len() * 4,
            crate::unicode_runtime_tables::casefold_sequences_hex(),
            4,
        ),
    ]
}

fn raw_data_object(
    symbol: &str,
    layout: &str,
    size: usize,
    value: String,
    alignment: usize,
) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: layout.to_string(),
        align: alignment,
        size: align(size, alignment),
        value,
    }
}

fn collect_string_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    let mut constants = HashMap::new();
    let mut types = HashMap::new();
    collect_string_values_from_ops_with_constants(ops, values, &mut constants, &mut types);
}

fn collect_string_values_from_ops_with_constants(
    ops: &[NirOp],
    values: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_string_values_from_value(code, values, constants, types);
            }
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types);
            }
            NirOp::StateAssign { value, .. } => {
                collect_string_values_from_value(value, values, constants, types);
            }
            NirOp::Assign { name, value } => {
                collect_string_values_from_value(value, values, constants, types);
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_string_values_from_value(value, values, constants, types);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values, constants, types);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    then_body,
                    values,
                    &mut then_constants,
                    &mut then_types,
                );
                collect_string_values_from_ops_with_constants(
                    else_body,
                    values,
                    &mut else_constants,
                    &mut else_types,
                );
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values, constants, types);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_string_values_from_value(value, values, constants, types);
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    collect_string_values_from_ops_with_constants(
                        &case.body,
                        values,
                        &mut case_constants,
                        &mut case_types,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_string_values_from_value(condition, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_string_values_from_value(start, values, constants, types);
                collect_string_values_from_value(end, values, constants, types);
                collect_string_values_from_value(step, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
                collect_string_values_from_value(condition, values, constants, types);
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                collect_string_values_from_value(iterable, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut trap_constants,
                    &mut trap_types,
                );
            }
        }
    }
}

fn collect_string_values_from_value(
    value: &NirValue,
    values: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) {
    if let Some(value) = static_string_value_with_constants(value, constants, types) {
        push_string_value(values, value);
    }
    if let NirValue::Call { target, args, .. }
    | NirValue::CallResult { target, args, .. }
    | NirValue::RuntimeCall { target, args, .. } = value
    {
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) = static_string_value_with_constants(&args[0], constants, types) {
                for grapheme in crate::unicode_backend::graphemes(&value) {
                    push_string_value(values, grapheme);
                }
            }
        }
        if target == "fs.pathJoin" && args.len() == 1 {
            push_string_value(values, "/".to_string());
        }
        if target == "fs.pathDirName" && args.len() == 1 {
            push_string_value(values, ".".to_string());
            push_string_value(values, "/".to_string());
        }
    }
    if value_may_return_invalid_format(value, constants, types) {
        push_string_value(values, ERR_INVALID_FORMAT_MESSAGE.to_string());
    }
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            push_string_value(values, value.clone());
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values, constants, types);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_string_values_from_value(value, values, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_values_from_value(target, values, constants, types);
            for update in updates {
                collect_string_values_from_value(&update.value, values, constants, types);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values, constants, types);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values, constants, types);
                collect_string_values_from_value(value, values, constants, types);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_string_values_from_value(target, values, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values, constants, types);
            collect_string_values_from_value(right, values, constants, types);
        }
        NirValue::Unary { operand, .. } => {
            collect_string_values_from_value(operand, values, constants, types)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_string_values_from_value(value, values, constants, types);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

impl CollectionTypeLayout {
    fn from_type(type_: &str) -> Option<Self> {
        if let Some(value_type) = type_.strip_prefix("List OF ") {
            return Some(Self {
                kind: COLLECTION_KIND_LIST,
                key_type_code: COLLECTION_TYPE_NONE,
                value_type_code: collection_type_code(value_type)?,
            });
        }
        let (key_type, value_type) = map_type_parts(type_)?;
        Some(Self {
            kind: COLLECTION_KIND_MAP,
            key_type_code: collection_type_code(&key_type)?,
            value_type_code: collection_type_code(&value_type)?,
        })
    }
}

fn collection_type_code(type_: &str) -> Option<usize> {
    match type_ {
        "Nothing" => None,
        "Boolean" => Some(COLLECTION_TYPE_BOOLEAN),
        "Byte" => Some(COLLECTION_TYPE_BYTE),
        "Integer" => Some(COLLECTION_TYPE_INTEGER),
        "Float" => Some(COLLECTION_TYPE_FLOAT),
        "Fixed" => Some(COLLECTION_TYPE_FIXED),
        "String" => Some(COLLECTION_TYPE_STRING),
        _ if type_.starts_with("List OF ") => Some(COLLECTION_TYPE_LIST),
        _ if type_.starts_with("Map OF ") => Some(COLLECTION_TYPE_MAP),
        _ => Some(COLLECTION_TYPE_OBJECT),
    }
}

/// Alignment, in bytes, of a packed collection payload identified by its compact
/// runtime type code. Mirrors `CodeBuilder::collection_payload_alignment` for
/// paths that carry the numeric type code rather than the type name: 8-byte
/// scalars, native collection/object pointers, and inline record/union slot
/// payloads require 8-byte alignment; 1-byte scalars and `String` bytes do not.
fn collection_payload_alignment_for_code(code: usize) -> usize {
    match code {
        COLLECTION_TYPE_INTEGER
        | COLLECTION_TYPE_FLOAT
        | COLLECTION_TYPE_FIXED
        | COLLECTION_TYPE_LIST
        | COLLECTION_TYPE_MAP
        | COLLECTION_TYPE_OBJECT => 8,
        _ => 1,
    }
}

fn value_may_return_invalid_format(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    (match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => match target.as_str() {
            "toInt" if args.len() == 1 => {
                static_type_name_with_types(&args[0], types).as_deref() != Some("Byte")
            }
            "toFloat" | "toFixed" | "isNumeric" => true,
            _ => false,
        },
        _ => false,
    }) || match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_return_invalid_format(arg, constants, types)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            value_may_return_invalid_format(value, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_return_invalid_format(target, constants, types)
                || updates
                    .iter()
                    .any(|update| value_may_return_invalid_format(&update.value, constants, types))
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_return_invalid_format(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_return_invalid_format(key, constants, types)
                || value_may_return_invalid_format(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_return_invalid_format(target, constants, types)
        }
        NirValue::Binary { op, left, right, .. } => {
            binary_may_promote_float_to_fixed(op, left, right, types)
                || value_may_return_invalid_format(left, constants, types)
                || value_may_return_invalid_format(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_may_return_invalid_format(operand, constants, types)
        }
        NirValue::Global { .. } => false,
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_may_return_invalid_format(value, constants, types)),
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::FunctionRef { .. } => false,
    }
}

fn push_string_value(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn local_constant_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if strings_package_static_string_value(target, args, constants, types).is_some() =>
        {
            strings_package_static_string_value(target, args, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        NirValue::Binary { op, .. } if op == "&" => {
            static_string_value_with_constants(value, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        _ => None,
    }
}

fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_string_value_with_constants(constant, constants, types)),
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types)
        }
        NirValue::Binary { op, left, right, .. } if op == "&" => {
            let left = static_string_value_with_constants(left, constants, types)?;
            let right = static_string_value_with_constants(right, constants, types)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

fn strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| static_string_value_with_constants(arg, constants, types))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
        }
        _ => None,
    }
}

fn binary_may_promote_float_to_fixed(
    op: &str,
    left: &NirValue,
    right: &NirValue,
    types: &HashMap<String, String>,
) -> bool {
    if !matches!(op, "+" | "-" | "*" | "/" | "MOD" | "^") {
        return false;
    }
    let Some(left_type) = static_type_name_with_types(left, types) else {
        return false;
    };
    let Some(right_type) = static_type_name_with_types(right, types) else {
        return false;
    };
    numeric_binary_result_type(op, &left_type, &right_type) == numeric::TYPE_FIXED
        && (left_type == numeric::TYPE_FLOAT || right_type == numeric::TYPE_FLOAT)
}

fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => types.get(name).cloned(),
        NirValue::Global { type_, .. } if !type_.is_empty() => Some(type_.clone()),
        NirValue::Global { .. } => None,
        NirValue::FunctionRef { type_, .. }
        | NirValue::Closure { type_, .. }
        | NirValue::Capture { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::UnionWrap { union_type, .. } => Some(union_type.clone()),
        NirValue::UnionExtract { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => match target.as_str() {
            "typeName" | "toString" => Some("String".to_string()),
            "len" | "toInt" => Some("Integer".to_string()),
            // Migrated find/mid/replace: strings:: returns Integer/String; the
            // collections:: List overloads return the list type and are resolved
            // by the precise type path, so only `find` (always Integer) is mapped
            // here (plan-01-functions.md §5).
            "collections.find" | "strings.find" => Some("Integer".to_string()),
            "strings.mid" | "strings.replace" => Some("String".to_string()),
            "strings.trim"
            | "strings.trimStart"
            | "strings.trimEnd"
            | "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.join" => Some("String".to_string()),
            "strings.graphemes" | "strings.split" => Some("List OF String".to_string()),
            "strings.startsWith" | "strings.endsWith" | "strings.contains" => {
                Some("Boolean".to_string())
            }
            "strings.byteLen" => Some("Integer".to_string()),
            "toFloat" => Some("Float".to_string()),
            "toFixed" => Some("Fixed".to_string()),
            "toByte" => Some("Byte".to_string()),
            "isNumeric" => Some("Boolean".to_string()),
            _ => None,
        },
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_type_name_with_types(value, types)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string))
            .or_else(|| static_type_name_with_types(value, types)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::Binary { op, left, right, .. } => {
            if matches!(
                op.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some("Boolean".to_string());
            }
            if op == "&" {
                return Some("String".to_string());
            }
            let left = static_type_name_with_types(left, types)?;
            let right = static_type_name_with_types(right, types)?;
            Some(numeric_binary_result_type(op, &left, &right).to_string())
        }
        NirValue::Unary { op, operand, .. } => {
            if op == "NOT" {
                Some("Boolean".to_string())
            } else {
                static_type_name_with_types(operand, types)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types)?;
            if member == "result" {
                if let Some(output_type) = builtins::thread::parent_thread_output(&target_type) {
                    return Some(format!("Result OF {output_type}"));
                }
            }
            let (key_type, value_type) = parse_map_entry_type(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type),
                "value" => Some(value_type),
                _ => None,
            }
        }
    }
}

fn static_primitive_text_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_primitive_text_with_constants(constant, constants)),
        _ => None,
    }
}

fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_collection_type(type_: &str) -> bool {
    type_.starts_with("List OF ") || type_.starts_with("Map OF ")
}

fn list_element_type(type_: &str) -> Option<String> {
    let element = type_.strip_prefix("List OF ")?;
    // A `List OF RES File` element stores and is read as the bare resource borrow
    // (`File`); the `RES` ownership-axis marker is not part of the value (§15.6).
    Some(strip_res_marker(element).to_string())
}

fn map_type_parts(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), strip_res_marker(value).to_string()))
}

/// Strip a `RES ` collection-element ownership-axis marker (`RES File` -> `File`).
fn strip_res_marker(type_: &str) -> &str {
    type_.strip_prefix("RES ").unwrap_or(type_)
}

fn callable_return_type(type_: &str) -> Option<String> {
    let (_, returns) = type_.rsplit_once(") AS ")?;
    Some(returns.to_string())
}

fn function_type_parts(type_: &str) -> Option<(Vec<String>, String)> {
    let rest = type_.strip_prefix("FUNC(")?;
    let (params, returns) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").map(str::to_string).collect()
    };
    Some((params, returns.to_string()))
}

fn builtin_function_symbol_for_type(name: &str, type_: &str) -> Option<String> {
    builtins::general::builtin_function_id_for_type(name, type_)?;
    Some(format!(
        "_mfb_builtin_{}_{}",
        nir::symbol_fragment(name),
        nir::symbol_fragment(type_)
    ))
}

fn builtin_function_refs(module: &NirModule) -> Vec<(String, String, String)> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for function in &module.functions {
        collect_builtin_function_refs_in_ops(&function.body, &mut refs, &mut seen);
    }
    refs
}

fn collect_builtin_function_refs_in_ops(
    ops: &[NirOp],
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } | NirOp::Fail { error: value } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_builtin_function_refs_in_value(code, refs, seen);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(then_body, refs, seen);
                collect_builtin_function_refs_in_ops(else_body, refs, seen);
            }
            NirOp::Match { value, cases } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
                for case in cases {
                    if let NirMatchPattern::Value(pattern) = &case.pattern {
                        collect_builtin_function_refs_in_value(pattern, refs, seen);
                    }
                    collect_builtin_function_refs_in_ops(&case.body, refs, seen);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_builtin_function_refs_in_value(start, refs, seen);
                collect_builtin_function_refs_in_value(end, refs, seen);
                collect_builtin_function_refs_in_value(step, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::DoUntil { body, condition } => {
                collect_builtin_function_refs_in_ops(body, refs, seen);
                collect_builtin_function_refs_in_value(condition, refs, seen);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_builtin_function_refs_in_value(iterable, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::Trap { body, .. } => {
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
        }
    }
}

fn collect_builtin_function_refs_in_value(
    value: &NirValue,
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    match value {
        NirValue::FunctionRef { name, type_ } => {
            if let Some(symbol) = builtin_function_symbol_for_type(name, type_) {
                let key = format!("{name}\0{type_}");
                if seen.insert(key) {
                    refs.push((name.clone(), type_.clone(), symbol));
                }
            }
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. } => {
            for arg in args {
                collect_builtin_function_refs_in_value(arg, refs, seen);
            }
        }
        NirValue::Constructor { args, .. } => {
            for value in args {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_builtin_function_refs_in_value(value, refs, seen);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
            for update in updates {
                collect_builtin_function_refs_in_value(&update.value, refs, seen);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_builtin_function_refs_in_value(key, refs, seen);
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::Binary { left, right, .. } => {
            collect_builtin_function_refs_in_value(left, refs, seen);
            collect_builtin_function_refs_in_value(right, refs, seen);
        }
        NirValue::Unary { operand, .. } => {
            collect_builtin_function_refs_in_value(operand, refs, seen);
        }
        NirValue::MemberAccess { target, .. } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. } => {}
    }
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

fn native_immediate_value(type_: &str, value: &str) -> Result<String, String> {
    match type_ {
        "Nothing" => Ok("0".to_string()),
        "Float" => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("invalid Float constant `{value}`"))?
            .to_bits()
            .to_string()),
        "Fixed" => Ok(fixed_raw_from_decimal(value)?.to_string()),
        _ => Ok(value.to_string()),
    }
}

fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    const SCALE: i128 = 1_i128 << 32;

    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Fixed constant `{value}`"));
    }
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Fixed constant `{value}`"))?
    };
    let mut fractional_value = 0_i128;
    if !fractional.is_empty() {
        let mut denominator = 1_i128;
        for digit in fractional.bytes() {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
            fractional_value = fractional_value
                .checked_mul(10)
                .and_then(|current| current.checked_add((digit - b'0') as i128))
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
            denominator = denominator
                .checked_mul(10)
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        }
        let scaled = fractional_value
            .checked_mul(SCALE)
            .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        fractional_value = scaled / denominator;
        if (scaled % denominator) * 2 >= denominator {
            fractional_value += 1;
        }
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Fixed constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| format!("Fixed constant `{value}` is out of range"))
}

fn join_json<T: ToCodeJson>(values: &[T], indent: usize) -> String {
    values
        .iter()
        .map(|value| value.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
fn checked_arena_used_after_alloc(
    block_base: u64,
    current_offset: u64,
    capacity: u64,
    size: u64,
    align: u64,
) -> Result<(u64, u64), u64> {
    let invalid_argument = ERR_INVALID_ARGUMENT_CODE
        .parse::<u64>()
        .expect("invalid argument code");
    let out_of_memory = ERR_OUT_OF_MEMORY_CODE
        .parse::<u64>()
        .expect("out of memory code");
    if align == 0 || !align.is_power_of_two() {
        return Err(invalid_argument);
    }
    let size = size.max(1);
    let payload_base = block_base
        .checked_add(ARENA_BLOCK_HEADER_SIZE as u64)
        .ok_or(out_of_memory)?;
    let raw = payload_base
        .checked_add(current_offset)
        .ok_or(out_of_memory)?;
    let mask = align - 1;
    let aligned = raw
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(out_of_memory)?;
    let end = aligned.checked_add(size).ok_or(out_of_memory)?;
    let used = end.checked_sub(payload_base).ok_or(out_of_memory)?;
    if used > capacity {
        return Err(out_of_memory);
    }
    Ok((aligned, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_rejects_invalid_alignment() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 0),
            Err(77050002)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 3),
            Err(77050002)
        );
    }

    #[test]
    fn arena_handles_zero_size_allocations() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 0, 8),
            Ok((0x1020, 1))
        );
    }

    #[test]
    fn arena_checks_alignment_rounding_and_capacity() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1003, 5, 128, 8, 16),
            Ok((0x1030, 21))
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 120, 128, 16, 16),
            Err(77010001)
        );
    }

    #[test]
    fn arena_checks_arithmetic_overflow() {
        assert_eq!(
            checked_arena_used_after_alloc(u64::MAX - 8, 0, 128, 8, 8),
            Err(77010001)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, u64::MAX, u64::MAX, 8),
            Err(77010001)
        );
    }
}
