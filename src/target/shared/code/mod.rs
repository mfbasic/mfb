use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::builtins;
use crate::bytecode::{self, NativeConst, NativePackageExport};
use crate::json_string;
use crate::numeric;

use super::nir::{self, NirFunction, NirMatchPattern, NirModule, NirOp, NirRecordUpdate, NirValue};
use super::plan::{CallKind, NativePlan};
use super::runtime;

const RESULT_OK_TAG: &str = "0";
const RESULT_ERR_TAG: &str = "1";
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
const ERR_UNSUPPORTED_CODE: &str = "77050007";
const ERR_UNSUPPORTED_MESSAGE: &str = "unsupported operation";
const ERR_UNSUPPORTED_SYMBOL: &str = "_mfb_str_error_unsupported";
const ERR_EOF_CODE: &str = "77020003";
const ERR_EOF_MESSAGE: &str = "end of file";
const ERR_EOF_SYMBOL: &str = "_mfb_str_error_eof";
const ERR_RESOURCE_CLOSED_CODE: &str = "77030004";
const ERR_RESOURCE_CLOSED_MESSAGE: &str = "resource closed";
const ERR_RESOURCE_CLOSED_SYMBOL: &str = "_mfb_str_error_resource_closed";
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
pub(crate) const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
const ARENA_DESTROY_SYMBOL: &str = "_mfb_arena_destroy";
const ARENA_STATE_REGISTER: &str = "x19";
const CLOSURE_ENV_REGISTER: &str = "x28";
const CLOSURE_OBJECT_SIZE: usize = 16;
const CLOSURE_OFFSET_CODE: usize = 0;
const CLOSURE_OFFSET_ENV: usize = 8;
const ENTRY_STACK_SIZE: usize = 112;
const ENTRY_GLOBALS_OFFSET: usize = ENTRY_STACK_SIZE;
const ARENA_STATE_SIZE: usize = 88;
const ARENA_CLEANUP_FAILURE_COUNT_OFFSET: usize = 64;
const ARENA_CLEANUP_FAILURE_CODE_OFFSET: usize = 72;
const ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET: usize = 80;
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
const EMPTY_STRING_SYMBOL: &str = "_mfb_str_empty";
const FS_MODE_TYPE_MASK: &str = "61440";
const FS_MODE_DIRECTORY: &str = "16384";
const FS_MODE_REGULAR: &str = "32768";
const FILE_OFFSET_FD: usize = 0;
const FILE_OFFSET_CLOSED: usize = 8;
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
    fn package_runtime_imports(&self, _spec: &runtime::RuntimeHelperSpec) -> Vec<CodeImport> {
        Vec::new()
    }
    fn preserves_link_register_in_runtime_helpers(&self) -> bool;
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
    active_cleanups: Vec<ActiveCleanup>,
    pending_result_slots: Option<PendingResultSlots>,
    error_arena_restore_slot: Option<usize>,
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
enum ActiveCleanup {
    Thread(ThreadCleanup),
    Resource(ResourceCleanup),
}

#[derive(Clone, Copy)]
struct PendingResultSlots {
    value: usize,
    tag: usize,
    message: usize,
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
    let mut platform_imports = native_plan
        .platform_imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect::<HashMap<_, _>>();
    let mut imports = native_plan
        .platform_imports
        .iter()
        .map(|import| CodeImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect::<Vec<_>>();
    let all_package_exports = package_native_exports(packages)?;
    let mut used_package_symbols = used_package_symbols(module, native_plan);
    let package_return_types = all_package_exports
        .iter()
        .map(|export| {
            Ok((
                export.name.clone(),
                native_type_name(export.return_type).to_string(),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    include_transitive_package_symbols(&all_package_exports, &mut used_package_symbols);
    let package_exports = all_package_exports
        .into_iter()
        .filter(|export| used_package_symbols.contains(&export.symbol))
        .collect::<Vec<_>>();
    let mut package_global_offsets = HashMap::new();
    let mut package_global_count = 0usize;
    let mut package_global_packages = HashSet::new();
    for export in &package_exports {
        if !package_global_packages.insert(export.package_name.clone()) {
            continue;
        }
        for index in 0..export.global_count {
            package_global_offsets.insert(
                (export.package_name.clone(), index as u32),
                ENTRY_GLOBALS_OFFSET + (module.globals.len() + package_global_count) * 8,
            );
            package_global_count += 1;
        }
    }
    let mut package_runtime_symbols = Vec::new();
    add_package_runtime_symbols(&mut package_runtime_symbols, &package_exports)?;
    add_platform_package_runtime_imports(
        platform,
        &mut platform_imports,
        &mut imports,
        &package_runtime_symbols,
    );
    if platform.target() == "macos-aarch64" && !package_runtime_symbols.is_empty() {
        add_macos_package_runtime_imports(
            &mut platform_imports,
            &mut imports,
            &package_runtime_symbols,
        );
    }
    let package_string_symbols = package_string_symbols(&package_exports);
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
    if module_requires_empty_string_constant(module)
        || package_exports_require_empty_string_constant(&package_exports)
    {
        data_objects.push(CodeDataObject {
            symbol: EMPTY_STRING_SYMBOL.to_string(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: 16,
            value: String::new(),
        });
    }
    if native_plan
        .runtime_symbols
        .iter()
        .any(|symbol| symbol.starts_with("_mfb_rt_fs_") || symbol.starts_with("_mfb_rt_thread_"))
        || package_requires_standard_error_messages(&package_exports)
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
    if package_requires_unicode_runtime_tables(&package_exports) {
        data_objects.extend(unicode_runtime_data_objects());
    }
    for ((export_symbol, const_index), symbol) in &package_string_symbols {
        let export = package_exports
            .iter()
            .find(|export| &export.symbol == export_symbol)
            .ok_or_else(|| format!("package export '{export_symbol}' does not resolve"))?;
        let Some(NativeConst::String(value)) = export.constants.get(*const_index) else {
            continue;
        };
        data_objects.push(CodeDataObject {
            symbol: symbol.clone(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: align(8 + value.len() + 1, 8),
            value: value.clone(),
        });
    }
    let type_model = TypeModel::from_module_and_packages(module, packages)?;
    let mut code_functions = Vec::new();
    let mut runtime_symbols = native_plan.runtime_symbols.clone();
    for symbol in package_runtime_symbols {
        if !runtime_symbols.iter().any(|existing| existing == &symbol) {
            runtime_symbols.push(symbol);
        }
    }
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

    if let Some(entry) = &module.entry {
        let entry_symbol = nir::function_symbol(&entry.name);
        code_functions.push(lower_program_entry(
            &entry_symbol,
            &entry.returns,
            entry.accepts_args,
            global_initializer_symbol.as_deref(),
            ENTRY_STACK_SIZE + (module.globals.len() + package_global_count) * 8,
            module.globals.len() + package_global_count,
            &platform_imports,
            platform,
            skip_entry_arena_destroy,
            module_may_record_cleanup_failure(module),
        )?);
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
    for export in &package_exports {
        code_functions.push(lower_package_export_function(
            export,
            &package_string_symbols,
            &package_global_offsets,
        )?);
    }
    code_functions.push(lower_arena_alloc(platform)?);
    code_functions.push(lower_arena_destroy(platform)?);
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
    for symbol in &runtime_symbols {
        code_functions.push(lower_runtime_helper(symbol, &platform_imports, platform)?);
    }
    if runtime_symbols
        .iter()
        .any(|symbol| symbol == "_mfb_rt_thread_thread_start")
    {
        code_functions.push(lower_thread_trampoline(&platform_imports, platform)?);
    }

    let plan = NativeCodePlan {
        target: module.target.clone(),
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
                "  \"arch\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
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
        if let Some(fields) = builtins::io::builtin_type_fields("TerminalSize") {
            record_fields.insert(
                "TerminalSize".to_string(),
                fields
                    .iter()
                    .map(|(name, type_)| ((*name).to_string(), (*type_).to_string()))
                    .collect(),
            );
        }
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
            for type_export in bytecode::read_package_type_exports(package)? {
                model.add_package_type_export(type_export);
            }
        }
        Ok(model)
    }

    fn add_package_type_export(&mut self, type_export: bytecode::BytecodeTypeExport) {
        match type_export.kind {
            bytecode::BytecodeExportKind::Type => {
                self.record_fields.insert(
                    type_export.name,
                    type_export
                        .fields
                        .into_iter()
                        .map(|field| (field.name, field.type_))
                        .collect(),
                );
            }
            bytecode::BytecodeExportKind::Enum => {
                for (index, member) in type_export.members.into_iter().enumerate() {
                    self.enum_members
                        .insert((type_export.name.clone(), member), index);
                }
            }
            bytecode::BytecodeExportKind::Union => {
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
            bytecode::BytecodeExportKind::Func | bytecode::BytecodeExportKind::Sub => {}
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

fn lower_program_entry(
    language_entry_symbol: &str,
    language_entry_returns: &str,
    language_entry_accepts_args: bool,
    global_initializer_symbol: Option<&str>,
    entry_stack_size: usize,
    global_slot_count: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    skip_arena_destroy: bool,
    emit_cleanup_failure_audit: bool,
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
            abi::store_u64("x31", ARENA_STATE_REGISTER, ARENA_CLEANUP_FAILURE_CODE_OFFSET),
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
    if let Some(symbol) = global_initializer_symbol {
        instructions.push(abi::branch_link(symbol));
        relocations.push(CodeRelocation {
            from: "_main".to_string(),
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
        from: "_main".to_string(),
        to: language_entry_symbol.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
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
            "_main",
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
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_integer_to_stderr(
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_SEPARATOR_SYMBOL,
        "_main",
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
        "_main",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    if emit_cleanup_failure_audit {
        emit_cleanup_failure_audit_report(
            "_main",
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
    if !skip_arena_destroy {
        instructions.extend([
            abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 32),
            abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 40),
            abi::store_u64(RESULT_ERROR_MESSAGE_REGISTER, ARENA_STATE_REGISTER, 48),
            abi::branch_link(ARENA_DESTROY_SYMBOL),
            abi::load_u64(abi::return_register(), ARENA_STATE_REGISTER, 32),
            abi::load_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 40),
            abi::load_u64(RESULT_ERROR_MESSAGE_REGISTER, ARENA_STATE_REGISTER, 48),
        ]);
        relocations.push(CodeRelocation {
            from: "_main".to_string(),
            to: ARENA_DESTROY_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
    }
    platform.emit_program_exit("_main", &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: "program.entry".to_string(),
        symbol: "_main".to_string(),
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
        abi::load_u64("x9", ARENA_STATE_REGISTER, ARENA_CLEANUP_FAILURE_COUNT_OFFSET),
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
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
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
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
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

fn package_native_exports(packages: &[PathBuf]) -> Result<Vec<NativePackageExport>, String> {
    let mut exports = Vec::new();
    for package in packages {
        exports.extend(
            bytecode::read_package_native_exports_with_available_packages(package, packages)?,
        );
    }
    Ok(exports)
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
        NirOp::While { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => body
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

fn package_exports_require_empty_string_constant(exports: &[NativePackageExport]) -> bool {
    fn type_id_requires_empty_string(
        export: &NativePackageExport,
        type_id: u32,
        seen: &mut HashSet<u32>,
    ) -> bool {
        if type_id == bytecode::TYPE_STRING {
            return true;
        }
        let Some(bytecode::NativePackageTypeInfo::Record(fields)) = export.type_info.get(&type_id)
        else {
            return false;
        };
        if !seen.insert(type_id) {
            return false;
        }
        let result = fields
            .iter()
            .any(|field| type_id_requires_empty_string(export, field.type_id, seen));
        seen.remove(&type_id);
        result
    }

    exports.iter().any(|export| {
        export.code.iter().any(|instruction| {
            instruction.opcode == bytecode::NATIVE_OPCODE_LOAD_DEFAULT
                && instruction.operands.get(1).is_some_and(|type_id| {
                    type_id_requires_empty_string(export, *type_id, &mut HashSet::new())
                })
        })
    })
}

fn package_string_symbols(exports: &[NativePackageExport]) -> HashMap<(String, usize), String> {
    let mut symbols = HashMap::new();
    for export in exports {
        for (index, constant) in export.constants.iter().enumerate() {
            if matches!(constant, NativeConst::String(_)) {
                symbols.insert(
                    (export.symbol.clone(), index),
                    format!("{}_str_{index}", export.symbol),
                );
            }
        }
    }
    symbols
}

fn native_type_name(type_: bytecode::NativeType) -> &'static str {
    match type_ {
        bytecode::NativeType::Nothing => "Nothing",
        bytecode::NativeType::Boolean => "Boolean",
        bytecode::NativeType::Byte => "Byte",
        bytecode::NativeType::Integer => "Integer",
        bytecode::NativeType::Float => "Float",
        bytecode::NativeType::Fixed => "Fixed",
        bytecode::NativeType::String => "String",
        bytecode::NativeType::FileHandle => "File",
        bytecode::NativeType::Result => "Result",
        bytecode::NativeType::Other => "Unknown",
    }
}

fn used_package_symbols(module: &NirModule, native_plan: &NativePlan) -> HashSet<String> {
    let mut symbols = native_plan
        .functions
        .iter()
        .flat_map(|function| function.calls.iter())
        .filter(|call| matches!(call.kind, CallKind::Import))
        .map(|call| call.symbol.clone())
        .collect::<HashSet<_>>();
    let package_import_symbols = module
        .imports
        .iter()
        .map(|import| (import.name.clone(), import.symbol.clone()))
        .collect::<HashMap<_, _>>();
    for function in &module.functions {
        collect_package_function_refs_in_ops(&function.body, &package_import_symbols, &mut symbols);
    }
    symbols
}

fn include_transitive_package_symbols(
    exports: &[NativePackageExport],
    symbols: &mut HashSet<String>,
) {
    let export_symbols = exports
        .iter()
        .map(|export| export.symbol.clone())
        .collect::<HashSet<_>>();
    let mut changed = true;
    while changed {
        changed = false;
        for export in exports {
            if !symbols.contains(&export.symbol) {
                continue;
            }
            for target in export.external_calls.values() {
                if export_symbols.contains(&target.symbol) && symbols.insert(target.symbol.clone())
                {
                    changed = true;
                }
            }
        }
    }
}

fn collect_package_function_refs_in_ops(
    ops: &[NirOp],
    package_import_symbols: &HashMap<String, String>,
    symbols: &mut HashSet<String>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_package_function_refs_in_value(value, package_import_symbols, symbols);
                }
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } | NirOp::Fail { error: value } => {
                collect_package_function_refs_in_value(value, package_import_symbols, symbols);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_package_function_refs_in_value(condition, package_import_symbols, symbols);
                collect_package_function_refs_in_ops(then_body, package_import_symbols, symbols);
                collect_package_function_refs_in_ops(else_body, package_import_symbols, symbols);
            }
            NirOp::Match { value, cases } => {
                collect_package_function_refs_in_value(value, package_import_symbols, symbols);
                for case in cases {
                    if let NirMatchPattern::Value(pattern) = &case.pattern {
                        collect_package_function_refs_in_value(
                            pattern,
                            package_import_symbols,
                            symbols,
                        );
                    }
                    collect_package_function_refs_in_ops(
                        &case.body,
                        package_import_symbols,
                        symbols,
                    );
                }
            }
            NirOp::While { condition, body } => {
                collect_package_function_refs_in_value(condition, package_import_symbols, symbols);
                collect_package_function_refs_in_ops(body, package_import_symbols, symbols);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_package_function_refs_in_value(iterable, package_import_symbols, symbols);
                collect_package_function_refs_in_ops(body, package_import_symbols, symbols);
            }
            NirOp::Trap { body, .. } => {
                collect_package_function_refs_in_ops(body, package_import_symbols, symbols);
            }
        }
    }
}

fn collect_package_function_refs_in_value(
    value: &NirValue,
    package_import_symbols: &HashMap<String, String>,
    symbols: &mut HashSet<String>,
) {
    match value {
        NirValue::FunctionRef { name, .. } => {
            if let Some(symbol) = package_import_symbols.get(name) {
                symbols.insert(symbol.clone());
            }
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_package_function_refs_in_value(value, package_import_symbols, symbols);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_package_function_refs_in_value(arg, package_import_symbols, symbols);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_package_function_refs_in_value(value, package_import_symbols, symbols);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_package_function_refs_in_value(target, package_import_symbols, symbols);
            for update in updates {
                collect_package_function_refs_in_value(
                    &update.value,
                    package_import_symbols,
                    symbols,
                );
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_package_function_refs_in_value(value, package_import_symbols, symbols);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_package_function_refs_in_value(key, package_import_symbols, symbols);
                collect_package_function_refs_in_value(value, package_import_symbols, symbols);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_package_function_refs_in_value(target, package_import_symbols, symbols);
        }
        NirValue::Binary { left, right, .. } => {
            collect_package_function_refs_in_value(left, package_import_symbols, symbols);
            collect_package_function_refs_in_value(right, package_import_symbols, symbols);
        }
        NirValue::Unary { operand, .. } => {
            collect_package_function_refs_in_value(operand, package_import_symbols, symbols);
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. } => {}
    }
}

fn add_package_runtime_symbols(
    runtime_symbols: &mut Vec<String>,
    exports: &[NativePackageExport],
) -> Result<(), String> {
    for export in exports {
        for instruction in &export.code {
            if let Some(symbol) = package_runtime_symbol(instruction)? {
                if !runtime_symbols.iter().any(|existing| existing == symbol) {
                    runtime_symbols.push(symbol.to_string());
                }
            }
        }
    }
    Ok(())
}

fn add_platform_package_runtime_imports(
    platform: &dyn CodegenPlatform,
    platform_imports: &mut HashMap<String, String>,
    imports: &mut Vec<CodeImport>,
    runtime_symbols: &[String],
) {
    for runtime_symbol in runtime_symbols {
        let Some(spec) = runtime::spec_for_symbol(runtime_symbol) else {
            continue;
        };
        for import in platform.package_runtime_imports(&spec) {
            platform_imports
                .entry(import.symbol.clone())
                .or_insert_with(|| import.library.clone());
            if !imports
                .iter()
                .any(|existing| existing.symbol == import.symbol)
            {
                imports.push(import);
            }
        }
    }
}

fn add_macos_package_runtime_imports(
    platform_imports: &mut HashMap<String, String>,
    imports: &mut Vec<CodeImport>,
    runtime_symbols: &[String],
) {
    let mut symbols = Vec::new();
    for runtime_symbol in runtime_symbols {
        let Some(spec) = runtime::spec_for_symbol(runtime_symbol) else {
            continue;
        };
        match spec.call {
            "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                symbols.push("_write");
            }
            "io.flush" | "io.flushError" => {
                symbols.extend(["_fsync", "___error"]);
            }
            "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                symbols.push("_read");
                if spec.call == "io.input" {
                    symbols.extend(["_write", "_fsync", "___error"]);
                }
            }
            "io.pollInput" => symbols.push("_poll"),
            "io.isInputTerminal" | "io.isOutputTerminal" | "io.isErrorTerminal" => {
                symbols.push("_isatty");
            }
            "io.terminalSize" => symbols.push("_ioctl"),
            "fs.exists" => symbols.push("_access"),
            "fs.fileExists" | "fs.directoryExists" => symbols.push("_stat"),
            "fs.currentDirectory" => symbols.push("_getcwd"),
            "fs.tempDirectory" => symbols.push("_confstr"),
            "fs.setCurrentDirectory" => {
                symbols.extend(["_chdir", "___error"]);
            }
            "fs.deleteFile" => {
                symbols.extend(["_unlink", "___error"]);
            }
            "fs.createDirectory" | "fs.createDirectories" => {
                symbols.extend(["_mkdir", "___error"]);
            }
            "fs.deleteDirectory" => {
                symbols.extend(["_rmdir", "___error"]);
            }
            "fs.listDirectory" => {
                symbols.extend(["_opendir", "_readdir", "_closedir", "___error"]);
            }
            "fs.open"
            | "fs.openFile"
            | "fs.openFileNoFollow"
            | "fs.createTempFile"
            | "fs.readText"
            | "fs.readBytes"
            | "fs.writeText"
            | "fs.writeBytes"
            | "fs.writeTextAtomic"
            | "fs.writeBytesAtomic"
            | "fs.appendText"
            | "fs.appendBytes"
            | "fs.readAll"
            | "fs.readAllBytes"
            | "fs.writeAll"
            | "fs.writeAllBytes"
            | "fs.close"
            | "fs.eof" => {
                symbols.extend([
                    "_open", "_read", "_write", "_close", "_fsync", "_lseek", "___error",
                ]);
                if spec.call == "fs.createTempFile" {
                    symbols.push("_getentropy");
                }
                if matches!(spec.call, "fs.writeTextAtomic" | "fs.writeBytesAtomic") {
                    symbols.extend(["_mkstemps", "_rename"]);
                }
            }
            "fs.canonicalPath" | "fs.isWithin" => {
                symbols.extend(["_realpath", "___error"]);
            }
            "thread.start" | "thread.isRunning" | "thread.waitFor" | "thread.cancel"
            | "thread.drop" | "thread.send" | "thread.poll" | "thread.read" | "thread.receive"
            | "thread.emit" | "thread.isCancelled" => {
                symbols.extend(thread_platform_symbols("macos-aarch64"))
            }
            _ => {}
        }
    }
    for symbol in symbols {
        if !platform_imports.contains_key(symbol) {
            platform_imports.insert(symbol.to_string(), "libSystem".to_string());
        }
        if !imports.iter().any(|import| import.symbol == symbol) {
            imports.push(CodeImport {
                library: "libSystem".to_string(),
                symbol: symbol.to_string(),
            });
        }
    }
}

fn package_requires_standard_error_messages(exports: &[NativePackageExport]) -> bool {
    exports.iter().any(|export| {
        export.code.iter().any(|instruction| {
            package_runtime_symbol(instruction)
                .ok()
                .flatten()
                .and_then(runtime::spec_for_symbol)
                .is_some()
        })
    })
}

fn package_requires_unicode_runtime_tables(exports: &[NativePackageExport]) -> bool {
    exports.iter().any(|export| {
        export.code.iter().any(|instruction| {
            package_runtime_symbol(instruction)
                .ok()
                .flatten()
                .and_then(runtime::spec_for_symbol)
                .is_some_and(|spec| {
                    matches!(
                        spec.call,
                        "strings.upper"
                            | "strings.lower"
                            | "strings.caseFold"
                            | "strings.normalizeNfc"
                            | "strings.graphemes"
                            | "strings.trim"
                            | "strings.trimStart"
                            | "strings.trimEnd"
                    )
                })
        })
    })
}

fn package_runtime_symbol(
    instruction: &bytecode::NativeInstruction,
) -> Result<Option<&'static str>, String> {
    let symbol = match instruction.opcode {
        bytecode::NATIVE_OPCODE_IO_WRITE => {
            let fd = native_operand(instruction, 2)?;
            let newline = native_operand(instruction, 3)?;
            match (fd, newline) {
                (1, 1) => "_mfb_rt_io_io_print",
                (1, 0) => "_mfb_rt_io_io_write",
                (2, 1) => "_mfb_rt_io_io_printError",
                (2, 0) => "_mfb_rt_io_io_writeError",
                _ => {
                    return Err(format!(
                    "native bytecode IO_WRITE uses unsupported fd/newline operands {fd}/{newline}"
                ))
                }
            }
        }
        bytecode::NATIVE_OPCODE_IO_FLUSH => match native_operand(instruction, 1)? {
            1 => "_mfb_rt_io_io_flush",
            2 => "_mfb_rt_io_io_flushError",
            fd => {
                return Err(format!(
                    "native bytecode IO_FLUSH uses unsupported fd operand {fd}"
                ))
            }
        },
        bytecode::NATIVE_OPCODE_IO_READ_LINE => {
            let prompt = native_operand(instruction, 1)?;
            if prompt == u32::MAX {
                "_mfb_rt_io_io_readLine"
            } else {
                "_mfb_rt_io_io_input"
            }
        }
        bytecode::NATIVE_OPCODE_IO_READ_CHAR => "_mfb_rt_io_io_readChar",
        bytecode::NATIVE_OPCODE_IO_READ_BYTE => "_mfb_rt_io_io_readByte",
        bytecode::NATIVE_OPCODE_IO_POLL_INPUT => "_mfb_rt_io_io_pollInput",
        bytecode::NATIVE_OPCODE_IO_IS_TERMINAL => match native_operand(instruction, 1)? {
            0 => "_mfb_rt_io_io_isInputTerminal",
            1 => "_mfb_rt_io_io_isOutputTerminal",
            2 => "_mfb_rt_io_io_isErrorTerminal",
            fd => {
                return Err(format!(
                    "native bytecode IO_IS_TERMINAL uses unsupported fd operand {fd}"
                ))
            }
        },
        bytecode::NATIVE_OPCODE_IO_TERMINAL_SIZE => "_mfb_rt_io_io_terminalSize",
        bytecode::NATIVE_OPCODE_FS_FILE_EXISTS => "_mfb_rt_fs_fs_fileExists",
        bytecode::NATIVE_OPCODE_FS_DIRECTORY_EXISTS => "_mfb_rt_fs_fs_directoryExists",
        bytecode::NATIVE_OPCODE_FS_EXISTS => "_mfb_rt_fs_fs_exists",
        bytecode::NATIVE_OPCODE_FS_READ_TEXT => "_mfb_rt_fs_fs_readText",
        bytecode::NATIVE_OPCODE_FS_WRITE_TEXT => "_mfb_rt_fs_fs_writeText",
        bytecode::NATIVE_OPCODE_FS_WRITE_TEXT_ATOMIC => "_mfb_rt_fs_fs_writeTextAtomic",
        bytecode::NATIVE_OPCODE_FS_APPEND_TEXT => "_mfb_rt_fs_fs_appendText",
        bytecode::NATIVE_OPCODE_FS_OPEN => "_mfb_rt_fs_fs_open",
        bytecode::NATIVE_OPCODE_FS_OPEN_NO_FOLLOW => "_mfb_rt_fs_fs_openFileNoFollow",
        bytecode::NATIVE_OPCODE_FS_CREATE_TEMP_FILE => "_mfb_rt_fs_fs_createTempFile",
        bytecode::NATIVE_OPCODE_FS_READ_LINE => "_mfb_rt_fs_fs_readLine",
        bytecode::NATIVE_OPCODE_FS_READ_ALL => "_mfb_rt_fs_fs_readAll",
        bytecode::NATIVE_OPCODE_FS_WRITE_ALL => "_mfb_rt_fs_fs_writeAll",
        bytecode::NATIVE_OPCODE_FS_CLOSE => "_mfb_rt_fs_fs_close",
        bytecode::NATIVE_OPCODE_FS_EOF => "_mfb_rt_fs_fs_eof",
        bytecode::NATIVE_OPCODE_FS_CANONICAL_PATH => "_mfb_rt_fs_fs_canonicalPath",
        bytecode::NATIVE_OPCODE_FS_IS_WITHIN => "_mfb_rt_fs_fs_isWithin",
        bytecode::NATIVE_OPCODE_FS_DELETE_FILE => "_mfb_rt_fs_fs_deleteFile",
        bytecode::NATIVE_OPCODE_FS_CREATE_DIRECTORY => "_mfb_rt_fs_fs_createDirectory",
        bytecode::NATIVE_OPCODE_FS_CREATE_DIRECTORIES => "_mfb_rt_fs_fs_createDirectories",
        bytecode::NATIVE_OPCODE_FS_DELETE_DIRECTORY => "_mfb_rt_fs_fs_deleteDirectory",
        bytecode::NATIVE_OPCODE_FS_LIST_DIRECTORY => "_mfb_rt_fs_fs_listDirectory",
        bytecode::NATIVE_OPCODE_FS_CURRENT_DIRECTORY => "_mfb_rt_fs_fs_currentDirectory",
        bytecode::NATIVE_OPCODE_FS_TEMP_DIRECTORY => "_mfb_rt_fs_fs_tempDirectory",
        bytecode::NATIVE_OPCODE_FS_SET_CURRENT_DIRECTORY => "_mfb_rt_fs_fs_setCurrentDirectory",
        bytecode::NATIVE_OPCODE_STRING_TRIM => "_mfb_rt_strings_strings_trim",
        bytecode::NATIVE_OPCODE_STRING_TRIM_START => "_mfb_rt_strings_strings_trimStart",
        bytecode::NATIVE_OPCODE_STRING_TRIM_END => "_mfb_rt_strings_strings_trimEnd",
        bytecode::NATIVE_OPCODE_STRING_UPPER => "_mfb_rt_strings_strings_upper",
        bytecode::NATIVE_OPCODE_STRING_LOWER => "_mfb_rt_strings_strings_lower",
        bytecode::NATIVE_OPCODE_STRING_CASE_FOLD => "_mfb_rt_strings_strings_caseFold",
        bytecode::NATIVE_OPCODE_STRING_NORMALIZE_NFC => "_mfb_rt_strings_strings_normalizeNfc",
        bytecode::NATIVE_OPCODE_STRING_GRAPHEMES => "_mfb_rt_strings_strings_graphemes",
        bytecode::NATIVE_OPCODE_STRING_STARTS_WITH => "_mfb_rt_strings_strings_startsWith",
        bytecode::NATIVE_OPCODE_STRING_ENDS_WITH => "_mfb_rt_strings_strings_endsWith",
        bytecode::NATIVE_OPCODE_STRING_CONTAINS => "_mfb_rt_strings_strings_contains",
        bytecode::NATIVE_OPCODE_STRING_SPLIT => "_mfb_rt_strings_strings_split",
        bytecode::NATIVE_OPCODE_STRING_JOIN => "_mfb_rt_strings_strings_join",
        bytecode::NATIVE_OPCODE_STRING_BYTE_LEN => "_mfb_rt_strings_strings_byteLen",
        bytecode::NATIVE_OPCODE_THREAD_START => "_mfb_rt_thread_thread_start",
        bytecode::NATIVE_OPCODE_THREAD_IS_RUNNING => "_mfb_rt_thread_thread_isRunning",
        bytecode::NATIVE_OPCODE_THREAD_WAIT_FOR => "_mfb_rt_thread_thread_waitFor",
        bytecode::NATIVE_OPCODE_THREAD_CANCEL => "_mfb_rt_thread_thread_cancel",
        bytecode::NATIVE_OPCODE_THREAD_SEND => "_mfb_rt_thread_thread_send",
        bytecode::NATIVE_OPCODE_THREAD_POLL => "_mfb_rt_thread_thread_poll",
        bytecode::NATIVE_OPCODE_THREAD_READ => "_mfb_rt_thread_thread_read",
        bytecode::NATIVE_OPCODE_THREAD_RECEIVE => "_mfb_rt_thread_thread_receive",
        bytecode::NATIVE_OPCODE_THREAD_EMIT => "_mfb_rt_thread_thread_emit",
        bytecode::NATIVE_OPCODE_THREAD_IS_CANCELLED => "_mfb_rt_thread_thread_isCancelled",
        _ => return Ok(None),
    };
    Ok(Some(symbol))
}

fn lower_package_runtime_call(
    export: &NativePackageExport,
    instruction: &bytecode::NativeInstruction,
    pc: usize,
    helper: &str,
    slot: &dyn Fn(u32) -> usize,
    scratch_base: usize,
    frame_size: usize,
    lr_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let spec = runtime::spec_for_symbol(helper)
        .ok_or_else(|| format!("package helper symbol '{helper}' has no runtime spec"))?;
    for index in 0..spec.abi.params.len() {
        let register = native_operand(instruction, index + 1)?;
        instructions.push(abi::load_u64(
            &abi::argument_register(index)?,
            abi::stack_pointer(),
            slot(register),
        ));
    }
    instructions.extend([
        abi::branch_link(helper),
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
    ]);
    if instruction.opcode == bytecode::NATIVE_OPCODE_THREAD_WAIT_FOR {
        let dst = native_operand(instruction, 0)?;
        if export
            .registers
            .get(dst as usize)
            .and_then(|register| export.type_info.get(&register.type_id))
            .is_some_and(|type_info| {
                matches!(type_info, bytecode::NativePackageTypeInfo::Result { .. })
            })
        {
            lower_package_raw_result_store(
                export,
                dst,
                pc,
                slot,
                scratch_base,
                frame_size,
                lr_offset,
                instructions,
                relocations,
            );
            relocations.push(internal_branch(&export.symbol, helper));
            return Ok(());
        }
    }
    let ok = format!("{}_pkg_runtime_ok_{pc}", export.symbol);
    instructions.extend([
        abi::branch_eq(&ok),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
        abi::label(&ok),
    ]);
    relocations.push(internal_branch(&export.symbol, helper));
    if spec.abi.returns != "Nothing" {
        let dst = native_operand(instruction, 0)?;
        instructions.push(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slot(dst),
        ));
    } else {
        let dst = native_operand(instruction, 0)?;
        instructions.extend([
            abi::move_immediate("x9", "Integer", "0"),
            abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
        ]);
    }
    Ok(())
}

fn lower_package_raw_result_store(
    export: &NativePackageExport,
    dst: u32,
    pc: usize,
    slot: &dyn Fn(u32) -> usize,
    scratch_base: usize,
    frame_size: usize,
    lr_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let tag_slot = scratch_base;
    let value_slot = scratch_base + 8;
    let message_slot = scratch_base + 16;
    let payload_slot = scratch_base + 24;
    let result_slot = scratch_base + 32;
    let wrap_error = format!("{}_pkg_raw_result_wrap_error_{pc}", export.symbol);
    let have_payload = format!("{}_pkg_raw_result_have_payload_{pc}", export.symbol);
    let error_alloc_ok = format!("{}_pkg_raw_result_error_alloc_ok_{pc}", export.symbol);
    let result_alloc_ok = format!("{}_pkg_raw_result_alloc_ok_{pc}", export.symbol);
    let allocation_failed = format!("{}_pkg_raw_result_alloc_failed_{pc}", export.symbol);

    instructions.extend([
        abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), tag_slot),
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), value_slot),
        abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ),
        abi::load_u64("x9", abi::stack_pointer(), tag_slot),
        abi::compare_immediate("x9", RESULT_OK_TAG),
        abi::branch_ne(&wrap_error),
        abi::load_u64("x9", abi::stack_pointer(), value_slot),
        abi::store_u64("x9", abi::stack_pointer(), payload_slot),
        abi::branch(&have_payload),
        abi::label(&wrap_error),
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&error_alloc_ok),
        abi::branch(&allocation_failed),
        abi::label(&error_alloc_ok),
        abi::load_u64("x9", abi::stack_pointer(), value_slot),
        abi::store_u64("x9", "x1", 0),
        abi::load_u64("x9", abi::stack_pointer(), message_slot),
        abi::store_u64("x9", "x1", 8),
        abi::store_u64("x1", abi::stack_pointer(), payload_slot),
        abi::label(&have_payload),
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&result_alloc_ok),
        abi::branch(&allocation_failed),
        abi::label(&result_alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), result_slot),
        abi::load_u64("x9", abi::stack_pointer(), tag_slot),
        abi::store_u64("x9", "x1", 0),
        abi::load_u64("x9", abi::stack_pointer(), payload_slot),
        abi::store_u64("x9", "x1", 8),
        abi::load_u64("x9", abi::stack_pointer(), result_slot),
        abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
        abi::branch(&format!("{}_pkg_raw_result_done_{pc}", export.symbol)),
        abi::label(&allocation_failed),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        &export.symbol,
        ERR_ALLOCATION_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
        abi::label(&format!("{}_pkg_raw_result_done_{pc}", export.symbol)),
    ]);
    relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
    relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
}

fn lower_package_call_result(
    export: &NativePackageExport,
    instruction: &bytecode::NativeInstruction,
    pc: usize,
    slot: &dyn Fn(u32) -> usize,
    frame_size: usize,
    lr_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let dst = native_operand(instruction, 0)?;
    let function_id = native_operand(instruction, 1)?;
    let target = export.external_calls.get(&function_id).ok_or_else(|| {
        format!(
            "package export '{}' calls unresolved package function id {function_id}",
            export.name
        )
    })?;
    for index in 2..instruction.operands.len() {
        let register = native_operand(instruction, index)?;
        instructions.push(abi::load_u64(
            &abi::argument_register(index - 2)?,
            abi::stack_pointer(),
            slot(register),
        ));
    }
    instructions.extend([
        abi::branch_link(&target.symbol),
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
    ]);
    let ok = format!("{}_pkg_call_ok_{pc}", export.symbol);
    instructions.extend([
        abi::branch_eq(&ok),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
        abi::label(&ok),
    ]);
    relocations.push(internal_branch(&export.symbol, &target.symbol));
    if target.return_type != bytecode::NativeType::Nothing {
        instructions.push(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slot(dst),
        ));
    } else {
        instructions.extend([
            abi::move_immediate("x9", "Integer", "0"),
            abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
        ]);
    }
    Ok(())
}

fn package_default_depth(export: &NativePackageExport) -> usize {
    fn depth_for_type(
        export: &NativePackageExport,
        type_id: u32,
        seen: &mut HashSet<u32>,
    ) -> usize {
        let Some(info) = export.type_info.get(&type_id) else {
            return 0;
        };
        let bytecode::NativePackageTypeInfo::Record(fields) = info else {
            return 0;
        };
        if !seen.insert(type_id) {
            return 0;
        }
        let depth = 1 + fields
            .iter()
            .map(|field| depth_for_type(export, field.type_id, seen))
            .max()
            .unwrap_or(0);
        seen.remove(&type_id);
        depth
    }

    export
        .registers
        .iter()
        .map(|register| depth_for_type(export, register.type_id, &mut HashSet::new()))
        .max()
        .unwrap_or(0)
}

fn emit_package_compare_values_branch(
    export: &NativePackageExport,
    type_id: u32,
    left: &str,
    right: &str,
    scratch_base: usize,
    depth: usize,
    equal_label: &str,
    not_equal_label: &str,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    match type_id {
        bytecode::TYPE_NOTHING => {
            instructions.push(abi::branch(equal_label));
        }
        bytecode::TYPE_BOOLEAN
        | bytecode::TYPE_BYTE
        | bytecode::TYPE_INTEGER
        | bytecode::TYPE_FIXED => {
            instructions.extend([
                abi::compare_registers(left, right),
                abi::branch_eq(equal_label),
                abi::branch(not_equal_label),
            ]);
        }
        bytecode::TYPE_FLOAT => {
            instructions.extend([
                abi::float_move_d_from_x("d0", left),
                abi::float_move_d_from_x("d1", right),
                abi::float_compare_d("d0", "d1"),
                abi::branch_eq(equal_label),
                abi::branch(not_equal_label),
            ]);
        }
        bytecode::TYPE_STRING => {
            let loop_label = format!(
                "{}_pkg_compare_string_loop_{}",
                export.symbol,
                instructions.len()
            );
            instructions.extend([
                abi::move_register("x4", left),
                abi::move_register("x5", right),
                abi::load_u64("x2", "x4", 0),
                abi::load_u64("x3", "x5", 0),
                abi::compare_registers("x2", "x3"),
                abi::branch_ne(not_equal_label),
                abi::add_immediate("x4", "x4", 8),
                abi::add_immediate("x5", "x5", 8),
                abi::label(&loop_label),
                abi::compare_immediate("x2", "0"),
                abi::branch_eq(equal_label),
                abi::load_u8("x6", "x4", 0),
                abi::load_u8("x7", "x5", 0),
                abi::compare_registers("x6", "x7"),
                abi::branch_ne(not_equal_label),
                abi::add_immediate("x4", "x4", 1),
                abi::add_immediate("x5", "x5", 1),
                abi::subtract_immediate("x2", "x2", 1),
                abi::branch(&loop_label),
            ]);
        }
        other => match export.type_info.get(&other) {
            Some(bytecode::NativePackageTypeInfo::Enum) => {
                instructions.extend([
                    abi::compare_registers(left, right),
                    abi::branch_eq(equal_label),
                    abi::branch(not_equal_label),
                ]);
            }
            Some(bytecode::NativePackageTypeInfo::Record(fields)) => {
                if fields.is_empty() {
                    instructions.push(abi::branch(equal_label));
                    return Ok(());
                }
                let left_slot = scratch_base + depth * 16;
                let right_slot = left_slot + 8;
                instructions.extend([
                    abi::store_u64(left, abi::stack_pointer(), left_slot),
                    abi::store_u64(right, abi::stack_pointer(), right_slot),
                ]);
                for field in fields {
                    let next_field = format!(
                        "{}_pkg_compare_record_next_{}",
                        export.symbol,
                        instructions.len()
                    );
                    instructions.extend([
                        abi::load_u64("x2", abi::stack_pointer(), left_slot),
                        abi::load_u64("x3", abi::stack_pointer(), right_slot),
                        abi::load_u64("x2", "x2", field.offset_slots * 8),
                        abi::load_u64("x3", "x3", field.offset_slots * 8),
                    ]);
                    emit_package_compare_values_branch(
                        export,
                        field.type_id,
                        "x2",
                        "x3",
                        scratch_base,
                        depth + 1,
                        &next_field,
                        not_equal_label,
                        instructions,
                    )?;
                    instructions.push(abi::label(&next_field));
                }
                instructions.push(abi::branch(equal_label));
            }
            Some(_) => {
                return Err(format!(
                    "package export '{}' cannot compare non-comparable type id {other}",
                    export.name
                ));
            }
            None => {
                return Err(format!(
                    "package export '{}' references unknown comparable type id {other}",
                    export.name
                ));
            }
        },
    }
    Ok(())
}

fn lower_package_equality(
    export: &NativePackageExport,
    instruction: &bytecode::NativeInstruction,
    pc: usize,
    slot: &dyn Fn(u32) -> usize,
    scratch_base: usize,
    instructions: &mut Vec<CodeInstruction>,
) -> Result<(), String> {
    let dst = native_operand(instruction, 0)?;
    let left = native_operand(instruction, 1)?;
    let right = native_operand(instruction, 2)?;
    let left_type = export
        .registers
        .get(left as usize)
        .ok_or_else(|| {
            format!(
                "package export '{}' instruction {pc} references missing register {left}",
                export.name
            )
        })?
        .type_id;
    let right_type = export
        .registers
        .get(right as usize)
        .ok_or_else(|| {
            format!(
                "package export '{}' instruction {pc} references missing register {right}",
                export.name
            )
        })?
        .type_id;
    if left_type != right_type {
        return Err(format!(
            "package export '{}' instruction {pc} compares mismatched type ids {left_type} and {right_type}",
            export.name
        ));
    }
    let equal_label = format!("{}_pkg_compare_equal_{pc}", export.symbol);
    let not_equal_label = format!("{}_pkg_compare_not_equal_{pc}", export.symbol);
    let done_label = format!("{}_pkg_compare_done_{pc}", export.symbol);
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), slot(left)),
        abi::load_u64("x10", abi::stack_pointer(), slot(right)),
    ]);
    emit_package_compare_values_branch(
        export,
        left_type,
        "x9",
        "x10",
        scratch_base,
        0,
        &equal_label,
        &not_equal_label,
        instructions,
    )?;
    let equal_value = if instruction.opcode == bytecode::NATIVE_OPCODE_EQUAL {
        "true"
    } else {
        "false"
    };
    let not_equal_value = if instruction.opcode == bytecode::NATIVE_OPCODE_EQUAL {
        "false"
    } else {
        "true"
    };
    instructions.extend([
        abi::label(&equal_label),
        abi::move_immediate("x11", "Boolean", equal_value),
        abi::branch(&done_label),
        abi::label(&not_equal_label),
        abi::move_immediate("x11", "Boolean", not_equal_value),
        abi::label(&done_label),
        abi::store_u64("x11", abi::stack_pointer(), slot(dst)),
    ]);
    Ok(())
}

fn package_collection_type_code(
    export: &NativePackageExport,
    type_id: u32,
) -> Result<usize, String> {
    if type_id == bytecode::TYPE_NOTHING {
        return Ok(COLLECTION_TYPE_NONE);
    }
    if type_id == bytecode::TYPE_BOOLEAN {
        return Ok(COLLECTION_TYPE_BOOLEAN);
    }
    if type_id == bytecode::TYPE_BYTE {
        return Ok(COLLECTION_TYPE_BYTE);
    }
    if type_id == bytecode::TYPE_INTEGER {
        return Ok(COLLECTION_TYPE_INTEGER);
    }
    if type_id == bytecode::TYPE_FLOAT {
        return Ok(COLLECTION_TYPE_FLOAT);
    }
    if type_id == bytecode::TYPE_FIXED {
        return Ok(COLLECTION_TYPE_FIXED);
    }
    if type_id == bytecode::TYPE_STRING {
        return Ok(COLLECTION_TYPE_STRING);
    }
    match export.type_info.get(&type_id) {
        Some(bytecode::NativePackageTypeInfo::List { .. }) => Ok(COLLECTION_TYPE_LIST),
        Some(bytecode::NativePackageTypeInfo::Map { .. }) => Ok(COLLECTION_TYPE_MAP),
        Some(bytecode::NativePackageTypeInfo::Record(_))
        | Some(bytecode::NativePackageTypeInfo::Enum)
        | Some(bytecode::NativePackageTypeInfo::Union(_)) => Ok(COLLECTION_TYPE_OBJECT),
        Some(bytecode::NativePackageTypeInfo::Result { .. })
        | Some(bytecode::NativePackageTypeInfo::Thread { .. })
        | Some(bytecode::NativePackageTypeInfo::ThreadWorker { .. })
        | Some(bytecode::NativePackageTypeInfo::Function)
        | Some(bytecode::NativePackageTypeInfo::Resource) => Err(format!(
            "package export '{}' collection payload type id {type_id} is not supported",
            export.name
        )),
        None => Err(format!(
            "package export '{}' collection payload type id {type_id} is unknown",
            export.name
        )),
    }
}

fn emit_package_empty_collection_default(
    export: &NativePackageExport,
    kind: usize,
    key_type_code: usize,
    value_type_code: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let alloc_ok = format!(
        "{}_pkg_default_collection_ok_{}",
        export.symbol,
        instructions.len()
    );
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &COLLECTION_HEADER_SIZE.to_string(),
        ),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        &export.symbol,
        ERR_ALLOCATION_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([abi::return_(), abi::label(&alloc_ok)]);
    relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::move_immediate("x8", "Byte", &kind.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x8", "Byte", &key_type_code.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x8", "Byte", &value_type_code.to_string()),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x8", "Byte", "1"),
        abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::move_immediate("x8", "Integer", "0"),
        abi::store_u64("x8", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x8", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        abi::move_register("x9", "x1"),
    ]);
}

fn emit_package_default_value(
    export: &NativePackageExport,
    type_id: u32,
    scratch_base: usize,
    depth: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if type_id == bytecode::TYPE_NOTHING
        || type_id == bytecode::TYPE_BOOLEAN
        || type_id == bytecode::TYPE_BYTE
        || type_id == bytecode::TYPE_INTEGER
        || type_id == bytecode::TYPE_FLOAT
        || type_id == bytecode::TYPE_FIXED
    {
        instructions.push(abi::move_immediate("x9", "Integer", "0"));
        return Ok(());
    }
    if type_id == bytecode::TYPE_STRING {
        instructions.push(abi::load_page_address("x9", EMPTY_STRING_SYMBOL));
        relocations.push(CodeRelocation {
            from: export.symbol.clone(),
            to: EMPTY_STRING_SYMBOL.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        instructions.push(abi::add_page_offset("x9", "x9", EMPTY_STRING_SYMBOL));
        relocations.push(CodeRelocation {
            from: export.symbol.clone(),
            to: EMPTY_STRING_SYMBOL.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        return Ok(());
    }

    match export.type_info.get(&type_id) {
        Some(bytecode::NativePackageTypeInfo::List { element_type }) => {
            emit_package_empty_collection_default(
                export,
                COLLECTION_KIND_LIST,
                COLLECTION_TYPE_NONE,
                package_collection_type_code(export, *element_type)?,
                instructions,
                relocations,
            );
            Ok(())
        }
        Some(bytecode::NativePackageTypeInfo::Map {
            key_type,
            value_type,
        }) => {
            emit_package_empty_collection_default(
                export,
                COLLECTION_KIND_MAP,
                package_collection_type_code(export, *key_type)?,
                package_collection_type_code(export, *value_type)?,
                instructions,
                relocations,
            );
            Ok(())
        }
        Some(bytecode::NativePackageTypeInfo::Record(fields)) => {
            let scratch_slot = scratch_base + depth * 8;
            let alloc_ok = format!(
                "{}_pkg_default_record_ok_{}",
                export.symbol,
                instructions.len()
            );
            instructions.extend([
                abi::move_immediate(
                    abi::return_register(),
                    "Integer",
                    &(fields.len() * 8).to_string(),
                ),
                abi::move_immediate("x1", "Integer", "8"),
                abi::branch_link(ARENA_ALLOC_SYMBOL),
                abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
                abi::branch_eq(&alloc_ok),
                abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
            ]);
            push_error_message_address(
                &export.symbol,
                ERR_ALLOCATION_SYMBOL,
                instructions,
                relocations,
            );
            instructions.extend([abi::return_(), abi::label(&alloc_ok)]);
            relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
            instructions.push(abi::store_u64("x1", abi::stack_pointer(), scratch_slot));
            for (index, field) in fields.iter().enumerate() {
                emit_package_default_value(
                    export,
                    field.type_id,
                    scratch_base,
                    depth + 1,
                    instructions,
                    relocations,
                )?;
                instructions.extend([
                    abi::load_u64("x10", abi::stack_pointer(), scratch_slot),
                    abi::store_u64("x9", "x10", 8 * index),
                ]);
            }
            instructions.push(abi::load_u64("x9", abi::stack_pointer(), scratch_slot));
            Ok(())
        }
        Some(bytecode::NativePackageTypeInfo::Enum)
        | Some(bytecode::NativePackageTypeInfo::Union(_))
        | Some(bytecode::NativePackageTypeInfo::Result { .. })
        | Some(bytecode::NativePackageTypeInfo::Thread { .. })
        | Some(bytecode::NativePackageTypeInfo::ThreadWorker { .. })
        | Some(bytecode::NativePackageTypeInfo::Function)
        | Some(bytecode::NativePackageTypeInfo::Resource) => Err(format!(
            "package export '{}' cannot materialize default value for type id {type_id}",
            export.name
        )),
        None => Err(format!(
            "package export '{}' references unknown default type id {type_id}",
            export.name
        )),
    }
}

fn lower_package_export_function(
    export: &NativePackageExport,
    string_symbols: &HashMap<(String, usize), String>,
    global_offsets: &HashMap<(String, u32), usize>,
) -> Result<CodeFunction, String> {
    let lr_offset = 0;
    let register_base = 8;
    let slot = |register: u32| register_base + register as usize * 8;
    let scratch_base = register_base + export.registers.len() * 8;
    let record_depth = package_default_depth(export);
    let scratch_slots = record_depth.max(record_depth * 2 + 2).max(5);
    let frame_size = align(scratch_base + scratch_slots * 8, 16);
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(frame_size)];
    let mut relocations = Vec::new();
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        lr_offset,
    ));
    for index in 0..export.param_count {
        instructions.push(abi::store_u64(
            &abi::argument_register(index)?,
            abi::stack_pointer(),
            slot(index as u32),
        ));
    }
    for (pc, instruction) in export.code.iter().enumerate() {
        if let Some(helper) = package_runtime_symbol(instruction)? {
            lower_package_runtime_call(
                export,
                instruction,
                pc,
                helper,
                &slot,
                scratch_base,
                frame_size,
                lr_offset,
                &mut instructions,
                &mut relocations,
            )?;
            continue;
        }
        match instruction.opcode {
            bytecode::NATIVE_OPCODE_LOAD_CONST => {
                let dst = native_operand(instruction, 0)?;
                let const_index = native_operand(instruction, 1)? as usize;
                let constant = export.constants.get(const_index).ok_or_else(|| {
                    format!(
                        "package export '{}' instruction {pc} references missing constant {const_index}",
                        export.name
                    )
                })?;
                emit_package_const(
                    export,
                    const_index,
                    constant,
                    string_symbols,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.push(abi::store_u64("x9", abi::stack_pointer(), slot(dst)));
            }
            bytecode::NATIVE_OPCODE_LOAD_DEFAULT => {
                let dst = native_operand(instruction, 0)?;
                let type_id = native_operand(instruction, 1)?;
                emit_package_default_value(
                    export,
                    type_id,
                    scratch_base,
                    0,
                    &mut instructions,
                    &mut relocations,
                )?;
                instructions.push(abi::store_u64("x9", abi::stack_pointer(), slot(dst)));
            }
            bytecode::NATIVE_OPCODE_LOAD_GLOBAL => {
                let dst = native_operand(instruction, 0)?;
                let global = native_operand(instruction, 1)?;
                let offset = package_global_offset(export, global_offsets, global)?;
                instructions.extend([
                    abi::load_u64("x9", ARENA_STATE_REGISTER, offset),
                    abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_STORE_GLOBAL => {
                let global = native_operand(instruction, 0)?;
                let src = native_operand(instruction, 1)?;
                let offset = package_global_offset(export, global_offsets, global)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(src)),
                    abi::store_u64("x9", ARENA_STATE_REGISTER, offset),
                ]);
            }
            bytecode::NATIVE_OPCODE_COPY | bytecode::NATIVE_OPCODE_MOVE => {
                let dst = native_operand(instruction, 0)?;
                let src = native_operand(instruction, 1)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(src)),
                    abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_UNWRAP_RESULT => {
                let dst = native_operand(instruction, 0)?;
                let src = native_operand(instruction, 1)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(src)),
                    abi::store_u64("x9", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_GENERAL_LEN => {
                let dst = native_operand(instruction, 0)?;
                let src = native_operand(instruction, 1)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(src)),
                    abi::load_u64("x10", "x9", 0),
                    abi::store_u64("x10", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_ADD => {
                let dst = native_operand(instruction, 0)?;
                let left = native_operand(instruction, 1)?;
                let right = native_operand(instruction, 2)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(left)),
                    abi::load_u64("x10", abi::stack_pointer(), slot(right)),
                    abi::add_registers("x11", "x9", "x10"),
                    abi::store_u64("x11", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_SUB => {
                let dst = native_operand(instruction, 0)?;
                let left = native_operand(instruction, 1)?;
                let right = native_operand(instruction, 2)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(left)),
                    abi::load_u64("x10", abi::stack_pointer(), slot(right)),
                    abi::subtract_registers("x11", "x9", "x10"),
                    abi::store_u64("x11", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_NEG => {
                let dst = native_operand(instruction, 0)?;
                let src = native_operand(instruction, 1)?;
                instructions.extend([
                    abi::load_u64("x9", abi::stack_pointer(), slot(src)),
                    abi::subtract_registers("x10", "x31", "x9"),
                    abi::store_u64("x10", abi::stack_pointer(), slot(dst)),
                ]);
            }
            bytecode::NATIVE_OPCODE_EQUAL | bytecode::NATIVE_OPCODE_NOT_EQUAL => {
                lower_package_equality(
                    export,
                    instruction,
                    pc,
                    &slot,
                    scratch_base,
                    &mut instructions,
                )?;
            }
            bytecode::NATIVE_OPCODE_CALL_RESULT => {
                lower_package_call_result(
                    export,
                    instruction,
                    pc,
                    &slot,
                    frame_size,
                    lr_offset,
                    &mut instructions,
                    &mut relocations,
                )?;
            }
            bytecode::NATIVE_OPCODE_RETURN_OK => {
                let value = native_operand(instruction, 0)?;
                if lower_package_return_union_variant(
                    export,
                    value,
                    pc,
                    &slot,
                    frame_size,
                    lr_offset,
                    &mut instructions,
                    &mut relocations,
                )? {
                    continue;
                }
                instructions.extend([
                    abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), slot(value)),
                    abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                    abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
                    abi::add_stack(frame_size),
                    abi::return_(),
                ]);
            }
            bytecode::NATIVE_OPCODE_CONSTRUCT_RECORD => {
                lower_package_construct_record(
                    export,
                    instruction,
                    pc,
                    &slot,
                    frame_size,
                    lr_offset,
                    &mut instructions,
                    &mut relocations,
                )?;
            }
            other => {
                return Err(format!(
                    "package export '{}' uses native bytecode opcode {other}, which is not lowered by the native package bridge",
                    export.name
                ));
            }
        }
    }
    if !instructions
        .iter()
        .any(|instruction| instruction.op == CodeOp::Ret)
    {
        instructions.extend([
            abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
            abi::add_stack(frame_size),
            abi::return_(),
        ]);
    }
    Ok(CodeFunction {
        name: format!("package.{}", export.name),
        symbol: export.symbol.clone(),
        params: (0..export.param_count)
            .map(|index| {
                Ok(CodeParam {
                    name: format!("arg{index}"),
                    type_: "Unknown".to_string(),
                    location: abi::argument_register(index)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        returns: "Unknown".to_string(),
        frame: CodeFrame {
            stack_size: frame_size,
            callee_saved: vec![abi::link_register().to_string()],
        },
        instructions,
        relocations,
        stack_slots: Vec::new(),
    })
}

fn lower_package_return_union_variant(
    export: &NativePackageExport,
    value: u32,
    pc: usize,
    slot: &dyn Fn(u32) -> usize,
    frame_size: usize,
    lr_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<bool, String> {
    let Some(bytecode::NativePackageTypeInfo::Union(union)) =
        export.type_info.get(&export.return_type_id)
    else {
        return Ok(false);
    };
    let value_type_id = export
        .registers
        .get(value as usize)
        .ok_or_else(|| {
            format!(
                "package export '{}' returns missing register {value}",
                export.name
            )
        })?
        .type_id;
    let Some(variant_name) = export.type_name_strings.get(&value_type_id).copied() else {
        return Ok(false);
    };
    let Some(variant) = union.variants.get(&variant_name) else {
        return Ok(false);
    };

    let union_slot = slot(value);
    let alloc_ok = format!("{}_pkg_return_union_ok_{pc}", export.symbol);
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &(union.size_slots * 8).to_string(),
        ),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        &export.symbol,
        ERR_ALLOCATION_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
        abi::label(&alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), slot(value)),
        abi::store_u64("x1", abi::stack_pointer(), union_slot),
        abi::move_immediate("x9", "Integer", &variant.tag.to_string()),
        abi::store_u64("x9", "x1", 0),
    ]);
    for field in &variant.fields {
        instructions.extend([
            abi::load_u64("x9", "x10", (field.offset_slots - 1) * 8),
            abi::load_u64("x11", abi::stack_pointer(), union_slot),
            abi::store_u64("x9", "x11", field.offset_slots * 8),
        ]);
    }
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), union_slot),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
    ]);
    relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
    Ok(true)
}

fn lower_package_construct_record(
    export: &NativePackageExport,
    instruction: &bytecode::NativeInstruction,
    pc: usize,
    slot: &dyn Fn(u32) -> usize,
    frame_size: usize,
    lr_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let dst = native_operand(instruction, 0)?;
    let type_id = native_operand(instruction, 1)?;
    let Some(bytecode::NativePackageTypeInfo::Record(fields)) = export.type_info.get(&type_id)
    else {
        return Err(format!(
            "package export '{}' constructs non-record type id {type_id}",
            export.name
        ));
    };
    if instruction.operands.len() != fields.len() + 2 {
        return Err(format!(
            "package export '{}' record constructor has {} field value(s), expected {}",
            export.name,
            instruction.operands.len().saturating_sub(2),
            fields.len()
        ));
    }
    let alloc_ok = format!("{}_pkg_construct_record_ok_{pc}", export.symbol);
    instructions.extend([
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            &(fields.len() * 8).to_string(),
        ),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        &export.symbol,
        ERR_ALLOCATION_SYMBOL,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::load_u64(abi::link_register(), abi::stack_pointer(), lr_offset),
        abi::add_stack(frame_size),
        abi::return_(),
        abi::label(&alloc_ok),
        abi::store_u64("x1", abi::stack_pointer(), slot(dst)),
    ]);
    for (index, field) in fields.iter().enumerate() {
        let operand = native_operand(instruction, 2 + index)?;
        instructions.extend([
            abi::load_u64("x9", abi::stack_pointer(), slot(dst)),
            abi::load_u64("x10", abi::stack_pointer(), slot(operand)),
            abi::store_u64("x10", "x9", field.offset_slots * 8),
        ]);
    }
    relocations.push(internal_branch(&export.symbol, ARENA_ALLOC_SYMBOL));
    Ok(())
}

fn native_operand(instruction: &bytecode::NativeInstruction, index: usize) -> Result<u32, String> {
    instruction.operands.get(index).copied().ok_or_else(|| {
        format!(
            "native bytecode opcode {} missing operand {index}",
            instruction.opcode
        )
    })
}

fn package_global_offset(
    export: &NativePackageExport,
    global_offsets: &HashMap<(String, u32), usize>,
    index: u32,
) -> Result<usize, String> {
    global_offsets
        .get(&(export.package_name.clone(), index))
        .copied()
        .ok_or_else(|| {
            format!(
                "package export '{}' references missing package global {index}",
                export.name
            )
        })
}

fn emit_package_const(
    export: &NativePackageExport,
    const_index: usize,
    constant: &NativeConst,
    string_symbols: &HashMap<(String, usize), String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    match constant {
        NativeConst::Nothing => instructions.push(abi::move_immediate("x9", "Integer", "0")),
        NativeConst::Boolean(value) => instructions.push(abi::move_immediate(
            "x9",
            "Boolean",
            if *value { "1" } else { "0" },
        )),
        NativeConst::Byte(value) => {
            instructions.push(abi::move_immediate("x9", "Byte", &value.to_string()))
        }
        NativeConst::Integer(value) => {
            instructions.push(abi::move_immediate("x9", "Integer", &value.to_string()))
        }
        NativeConst::Fixed(value) => {
            instructions.push(abi::move_immediate("x9", "Integer", &value.to_string()))
        }
        NativeConst::String(_) => {
            let symbol = string_symbols
                .get(&(export.symbol.clone(), const_index))
                .ok_or_else(|| {
                    format!(
                        "package export '{}' string constant {const_index} has no data symbol",
                        export.name
                    )
                })?;
            instructions.push(abi::load_page_address("x9", symbol));
            relocations.push(CodeRelocation {
                from: export.symbol.clone(),
                to: symbol.clone(),
                kind: "page21".to_string(),
                binding: "data".to_string(),
                library: None,
            });
            instructions.push(abi::add_page_offset("x9", "x9", symbol));
            relocations.push(CodeRelocation {
                from: export.symbol.clone(),
                to: symbol.clone(),
                kind: "pageoff12".to_string(),
                binding: "data".to_string(),
                library: None,
            });
        }
        NativeConst::Float(_) | NativeConst::Other => {
            return Err(format!(
                "package export '{}' uses unsupported native constant kind",
                export.name
            ));
        }
    }
    Ok(())
}

fn lower_runtime_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    match spec.call {
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let (frame, instructions, relocations) = lower_io_write_helper(
                symbol,
                platform_imports,
                platform,
                matches!(spec.call, "io.printError" | "io.writeError"),
                matches!(spec.call, "io.print" | "io.printError"),
            )?;
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
            let (frame, instructions, relocations) = lower_io_flush_helper(
                symbol,
                platform_imports,
                platform,
                matches!(spec.call, "io.flushError"),
            )?;
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
            let (frame, instructions, relocations) = lower_io_read_line_helper(
                symbol,
                platform_imports,
                platform,
                spec.call == "io.input",
            )?;
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
                lower_io_read_char_helper(symbol, platform_imports, platform)?;
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
                lower_io_read_byte_helper(symbol, platform_imports, platform)?;
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
            let (frame, instructions, relocations) =
                lower_io_is_terminal_helper(symbol, platform_imports, platform, fd)?;
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
        "io.terminalSize" => {
            let (frame, instructions, relocations) =
                lower_io_terminal_size_helper(symbol, platform_imports, platform)?;
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
        | "thread.emit" | "thread.isCancelled" => {
            let (frame, instructions, relocations) =
                lower_thread_helper(symbol, spec.call, platform_imports, platform)?;
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
        active_cleanups: Vec::new(),
        pending_result_slots: None,
        error_arena_restore_slot: None,
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

const THREAD_BLOCK_SIZE: usize = 96;
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

fn thread_platform_symbols(target: &str) -> Vec<&'static str> {
    if target == "macos-aarch64" {
        vec![
            "_pthread_create",
            "_pthread_detach",
            "_pthread_mutex_init",
            "_pthread_mutex_lock",
            "_pthread_mutex_unlock",
            "_pthread_cond_init",
            "_pthread_cond_wait",
            "_pthread_cond_timedwait",
            "_pthread_cond_signal",
            "_pthread_cond_broadcast",
            "_clock_gettime",
        ]
    } else {
        vec![
            "pthread_create",
            "pthread_detach",
            "pthread_mutex_init",
            "pthread_mutex_lock",
            "pthread_mutex_unlock",
            "pthread_cond_init",
            "pthread_cond_wait",
            "pthread_cond_timedwait",
            "pthread_cond_signal",
            "pthread_cond_broadcast",
            "clock_gettime",
        ]
    }
}

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
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    match call {
        "thread.start" => lower_thread_start_helper(symbol, platform_imports, platform),
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
        "thread.isCancelled" => Ok(thread_is_cancelled_helper()),
        _ => Err(format!("native thread helper does not implement {call}")),
    }
}

fn lower_thread_start_helper(
    symbol: &str,
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
        abi::store_u64("x31", "x9", THREAD_OFFSET_INBOUND_QUEUE),
        abi::store_u64("x31", "x9", THREAD_OFFSET_OUTBOUND_QUEUE),
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
                abi::store_u64(
                    RESULT_ERROR_MESSAGE_REGISTER,
                    abi::stack_pointer(),
                    ERROR_OFFSET,
                ),
                abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), VALUE_OFFSET),
                abi::store_u64(RESULT_TAG_REGISTER, abi::stack_pointer(), TAG_OFFSET),
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
        instructions.extend([
            abi::compare_registers("x20", "x0"),
            abi::branch_ne(&invalid),
        ]);
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
            abi::compare_registers("x20", "x0"),
            abi::branch_ne(&invalid),
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

fn lower_io_read_byte_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 32;
    const LR_OFFSET: usize = 0;
    const BYTE_OFFSET: usize = 8;
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

fn lower_io_terminal_size_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 64;
    const LR_OFFSET: usize = 48;
    const WINSIZE_OFFSET: usize = 24;
    const ROW_OFFSET: usize = 32;
    const COL_OFFSET: usize = 40;
    const LINUX_TIOCGWINSZ: &str = "21523";
    const DARWIN_TIOCGWINSZ: &str = "1074295912";
    let ioctl_error = format!("{symbol}_ioctl_error");
    let alloc_ok = format!("{symbol}_alloc_ok");
    let alloc_error = format!("{symbol}_alloc_error");
    let done = format!("{symbol}_done");

    let request = if platform.target() == "macos-aarch64" {
        DARWIN_TIOCGWINSZ
    } else {
        LINUX_TIOCGWINSZ
    };

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            LR_OFFSET,
        ));
    }
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate("x1", "Integer", request),
        abi::add_immediate("x2", abi::stack_pointer(), WINSIZE_OFFSET),
    ]);
    platform.emit_terminal_size(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&ioctl_error),
        abi::load_u16("x10", abi::stack_pointer(), WINSIZE_OFFSET),
        abi::load_u16("x11", abi::stack_pointer(), WINSIZE_OFFSET + 2),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&ioctl_error),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&ioctl_error),
        abi::store_u64("x10", abi::stack_pointer(), ROW_OFFSET),
        abi::store_u64("x11", abi::stack_pointer(), COL_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate("x1", "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq(&alloc_ok),
        abi::branch(&alloc_error),
        abi::label(&alloc_ok),
        abi::load_u64("x10", abi::stack_pointer(), ROW_OFFSET),
        abi::load_u64("x11", abi::stack_pointer(), COL_OFFSET),
        abi::store_u64("x11", "x1", 0),
        abi::store_u64("x10", "x1", 8),
        abi::move_register(RESULT_VALUE_REGISTER, "x1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&ioctl_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_UNSUPPORTED_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_UNSUPPORTED_SYMBOL,
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
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 64;
    const LR_OFFSET: usize = 0;
    const BYTES_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const RESULT_OFFSET: usize = 24;
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
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const BUFFER_OFFSET: usize = 8;
    const CAPACITY_OFFSET: usize = 16;
    const LENGTH_OFFSET: usize = 24;
    const SEQ_LEN_OFFSET: usize = 32;
    const RESULT_OFFSET: usize = 40;
    const BYTES_OFFSET: usize = 48;
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
    let err_not_found = format!("{symbol}_err_not_found");
    let err_access_denied = format!("{symbol}_err_access_denied");
    let err_already_exists = format!("{symbol}_err_already_exists");
    let err_not_empty = format!("{symbol}_err_not_empty");
    let err_output = format!("{symbol}_err_output");
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
    instructions.extend([
        abi::compare_immediate("x9", "2"),
        abi::branch_eq(&err_not_found),
        abi::compare_immediate("x9", "13"),
        abi::branch_eq(&err_access_denied),
        abi::compare_immediate("x9", "17"),
        abi::branch_eq(&err_already_exists),
        abi::compare_immediate("x9", "39"),
        abi::branch_eq(&err_not_empty),
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
        abi::label(&err_already_exists),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_ALREADY_EXISTS_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(
        symbol,
        ERR_ALREADY_EXISTS_SYMBOL,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::branch(&done),
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
        abi::move_immediate(abi::return_register(), "Integer", "16"),
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
        abi::move_immediate(abi::return_register(), "Integer", "16"),
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
    emit_errno_error_mapping(symbol, &mut instructions, &mut relocations, &done);
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
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STRING_OFFSET),
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
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STRING_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
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
        abi::move_immediate(abi::return_register(), "Integer", "16"),
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
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), RESULT_OFFSET),
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

mod builder_collection_layout;
mod builder_collection_queries;
mod builder_collection_updates;
mod builder_control;
mod builder_conversions;
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
            CodeOp::StrU64 | CodeOp::StrU8 => &["src", "base", "offset"],
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
    if module_uses_call(module, "io.terminalSize") {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
    }
    if module_uses_call(module, "find")
        || module_uses_call(module, "mid")
        || module_uses_call(module, "get")
        || module_uses_call(module, "append")
        || module_uses_call(module, "prepend")
        || module_uses_call(module, "insert")
        || module_uses_call(module, "transform")
        || module_uses_call(module, "filter")
        || module_uses_call(module, "removeAt")
        || module_uses_call(module, "set")
    {
        push_string_value(&mut values, ERR_INDEX_OUT_OF_RANGE_MESSAGE.to_string());
    }
    if module_uses_call(module, "find") || module_uses_call(module, "get") {
        push_string_value(&mut values, ERR_NOT_FOUND_MESSAGE.to_string());
    }
    if module_uses_call(module, "toString") {
        push_string_value(&mut values, "TRUE".to_string());
        push_string_value(&mut values, "FALSE".to_string());
        push_string_value(&mut values, FLOAT_TO_STRING_FORMAT.to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
    }
    for value in [ENTRY_ERROR_PREFIX, ENTRY_ERROR_SEPARATOR, ENTRY_ERROR_NEWLINE] {
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
        } => {
            ops_may_record_cleanup_failure(then_body)
                || ops_may_record_cleanup_failure(else_body)
        }
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| ops_may_record_cleanup_failure(&case.body)),
        NirOp::While { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_may_record_cleanup_failure(body),
        NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::Return { .. }
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
            NirOp::Fail { error } => value_may_emit_float_arithmetic_error(error, locals),
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
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
            NirOp::While { condition, body } => {
                value_may_emit_float_arithmetic_error(condition, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
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
        NirValue::Binary { op, left, right } => {
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
        NirValue::Unary { op, operand } => {
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
        NirValue::Binary { op, left, right } => static_nir_value_type(left, locals)
            .zip(static_nir_value_type(right, locals))
            .map(|(left_type, right_type)| {
                numeric_binary_result_type(op, &left_type, &right_type).to_string()
            }),
        NirValue::Unary { operand, .. } => static_nir_value_type(operand, locals),
        NirValue::Call { target, args } | NirValue::CallResult { target, args } => {
            let arg_types = args
                .iter()
                .map(|arg| static_nir_value_type(arg, locals))
                .collect::<Option<Vec<_>>>()?;
            builtins::general::resolve_call(target, &arg_types)
                .map(|call| call.return_type.into_owned())
                .or_else(|| builtins::call_return_type_name(target).map(str::to_string))
        }
        NirValue::ResultIsOk { .. } => Some("Boolean".to_string()),
        NirValue::ResultValue { value } => static_nir_value_type(value, locals)
            .and_then(|type_| type_.strip_prefix("Result OF ").map(str::to_string)),
        NirValue::ResultError { .. } => Some("Error".to_string()),
        NirValue::MemberAccess { target, member } => {
            let target_type = static_nir_value_type(target, locals)?;
            if member == "result" {
                builtins::thread::thread_output(&target_type)
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
        NirOp::Fail { error } => value_uses_call(error, target),
        NirOp::Assign { value, .. } | NirOp::Eval { value } => value_uses_call(value, target),
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
        NirOp::While { condition, body } => {
            value_uses_call(condition, target) || ops_use_call(body, target)
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_call(iterable, target) || ops_use_call(body, target)
        }
        NirOp::Trap { body, .. } => ops_use_call(body, target),
    })
}

fn value_uses_call(value: &NirValue, target: &str) -> bool {
    match value {
        NirValue::Call { target: call, args }
        | NirValue::CallResult { target: call, args }
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
        NirOp::Fail { error } => value_uses_type_name(error),
        NirOp::Assign { value, .. } | NirOp::Eval { value } => value_uses_type_name(value),
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
        NirOp::While { condition, body } => {
            value_uses_type_name(condition) || ops_use_type_name(body)
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
            NirOp::Fail { error } => collect_type_name_values_from_value(error, values),
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
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
            NirOp::While { condition, body } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(body, values);
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
            NirOp::While { condition, body } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
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
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
        | NirValue::RuntimeCall { target, args, .. } => {
            matches!(
                target.as_str(),
                "strings.upper"
                    | "strings.lower"
                    | "strings.caseFold"
                    | "strings.normalizeNfc"
                    | "strings.graphemes"
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
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types);
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
            NirOp::While { condition, body } => {
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
    if let NirValue::Call { target, args }
    | NirValue::CallResult { target, args }
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

fn value_may_return_invalid_format(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    (match value {
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
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
        NirValue::Binary { op, left, right } => {
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
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
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
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
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
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
        | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types)
        }
        NirValue::Call { target, args }
        | NirValue::CallResult { target, args }
        | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types)
        }
        NirValue::Binary { op, left, right } if op == "&" => {
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
            "replace" | "typeName" | "toString" => Some("String".to_string()),
            "find" | "len" | "toInt" => Some("Integer".to_string()),
            "mid" => Some("String".to_string()),
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
        NirValue::Binary { op, left, right } => {
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
        NirValue::Unary { op, operand } => {
            if op == "NOT" {
                Some("Boolean".to_string())
            } else {
                static_type_name_with_types(operand, types)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types)?;
            if member == "result" {
                if let Some(output_type) = builtins::thread::thread_output(&target_type) {
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
    type_.strip_prefix("List OF ").map(str::to_string)
}

fn map_type_parts(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
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
            NirOp::Assign { value, .. } | NirOp::Eval { value } | NirOp::Fail { error: value } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
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
            NirOp::While { condition, body } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
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

fn integer_constant_value(value: &NirValue) -> Option<i64> {
    match value {
        NirValue::Const { type_, value } if type_ == "Integer" => value.parse::<i64>().ok(),
        NirValue::Unary { op, operand } if op == "-" => {
            integer_constant_value(operand).and_then(i64::checked_neg)
        }
        _ => None,
    }
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

fn string_constant_value(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value),
        _ => None,
    }
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

    #[test]
    fn package_runtime_symbol_distinguishes_input_from_readline() {
        let read_line = bytecode::NativeInstruction {
            opcode: bytecode::NATIVE_OPCODE_IO_READ_LINE,
            operands: vec![0, u32::MAX],
        };
        let input = bytecode::NativeInstruction {
            opcode: bytecode::NATIVE_OPCODE_IO_READ_LINE,
            operands: vec![0, 7],
        };
        assert_eq!(
            package_runtime_symbol(&read_line).expect("readLine symbol"),
            Some("_mfb_rt_io_io_readLine")
        );
        assert_eq!(
            package_runtime_symbol(&input).expect("input symbol"),
            Some("_mfb_rt_io_io_input")
        );
    }

    #[test]
    fn package_runtime_symbol_maps_io_flush_by_fd() {
        for (fd, expected) in [(1, "_mfb_rt_io_io_flush"), (2, "_mfb_rt_io_io_flushError")] {
            let instruction = bytecode::NativeInstruction {
                opcode: bytecode::NATIVE_OPCODE_IO_FLUSH,
                operands: vec![0, fd],
            };
            assert_eq!(
                package_runtime_symbol(&instruction).expect("flush helper"),
                Some(expected)
            );
        }
    }

    #[test]
    fn package_runtime_symbol_rejects_invalid_io_flush_fd() {
        let instruction = bytecode::NativeInstruction {
            opcode: bytecode::NATIVE_OPCODE_IO_FLUSH,
            operands: vec![0, 3],
        };
        assert_eq!(
            package_runtime_symbol(&instruction),
            Err("native bytecode IO_FLUSH uses unsupported fd operand 3".to_string())
        );
    }

    #[test]
    fn package_runtime_symbol_maps_io_is_terminal_by_fd() {
        for (fd, expected) in [
            (0, "_mfb_rt_io_io_isInputTerminal"),
            (1, "_mfb_rt_io_io_isOutputTerminal"),
            (2, "_mfb_rt_io_io_isErrorTerminal"),
        ] {
            let instruction = bytecode::NativeInstruction {
                opcode: bytecode::NATIVE_OPCODE_IO_IS_TERMINAL,
                operands: vec![0, fd],
            };
            assert_eq!(
                package_runtime_symbol(&instruction).expect("terminal helper"),
                Some(expected)
            );
        }
    }
}
