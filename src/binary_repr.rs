use crate::builtins;
use crate::ir::{IrFunction, IrOp, IrProject, IrType, IrValue};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const SECTION_MANIFEST: u16 = 1;
const SECTION_STRING_POOL: u16 = 2;
const SECTION_TYPE_TABLE: u16 = 3;
const SECTION_CONST_POOL: u16 = 4;
const SECTION_IMPORT_TABLE: u16 = 5;
const SECTION_EXPORT_TABLE: u16 = 6;
const SECTION_GLOBAL_TABLE: u16 = 7;
const SECTION_FUNCTION_TABLE: u16 = 8;
const SECTION_RESOURCE_TABLE: u16 = 11;
const SECTION_ABI_INDEX: u16 = 15;
/// Structured Binary Representation payload section. Replaces the old flat code section as
/// the carrier of function bodies; see `crate::ir::encode_binary_repr`.
const SECTION_BINARY_REPR: u16 = 16;

/// MFPC container major version. Bumped to 2 for the clean break to the
/// structured Binary Representation payload — the reader rejects the old flat (v1) layout.
const MFPC_MAJOR_VERSION: u16 = 2;

const ABI_FORMAT_VERSION: u16 = 1;
const ABI_HASH_LEN: usize = 32;

pub(crate) const TYPE_NOTHING: u32 = 1;
pub(crate) const TYPE_BOOLEAN: u32 = 2;
pub(crate) const TYPE_INTEGER: u32 = 3;
pub(crate) const TYPE_FLOAT: u32 = 4;
pub(crate) const TYPE_FIXED: u32 = 5;
pub(crate) const TYPE_STRING: u32 = 6;
pub(crate) const TYPE_BYTE: u32 = 7;
pub(crate) const TYPE_ERROR: u32 = 8;
pub(crate) const TYPE_TERMINAL_SIZE: u32 = 9;
pub(crate) const TYPE_FILE_HANDLE: u32 = 0xffff_ff00;
pub(crate) const TYPE_SOCKET_HANDLE: u32 = 0xffff_feff;
pub(crate) const TYPE_LISTENER_HANDLE: u32 = 0xffff_fefe;
const FIRST_TABLE_TYPE_ID: u32 = 10;

const FUNCTION_BINARY_REPR: u16 = 1;

const FUNCTION_FLAG_ISOLATED: u16 = 1 << 2;
const FUNCTION_FLAG_PRIVATE: u16 = 1 << 1;
const FUNCTION_FLAG_SUB: u16 = 1 << 3;
const FUNCTION_FLAG_RETURNS_NOTHING: u16 = 1 << 5;

pub fn write_binary_repr_hex(
    project_dir: &Path,
    ir: &IrProject,
    version: &str,
) -> Result<PathBuf, String> {
    let metadata = BinaryReprMetadata::new(ir.name.clone(), version.to_string());
    let bytes = build_binary_repr_bytes(ir, &metadata)?;
    let hex_path = project_dir.join(format!("{}.hex", ir.name));
    fs::write(&hex_path, hex_dump(&bytes))
        .map_err(|err| format!("failed to write '{}': {err}", hex_path.display()))?;
    Ok(hex_path)
}

pub fn build_binary_repr_bytes(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
) -> Result<Vec<u8>, String> {
    Ok(lower_project(ir, metadata)?.encode())
}

pub fn build_package_binary_repr_bytes(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    packages: &[PathBuf],
) -> Result<Vec<u8>, String> {
    Ok(lower_package_project(ir, metadata, packages)?.encode())
}

#[derive(Clone)]
pub struct BinaryReprMetadata {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub ident_key: String,
    pub ident_fingerprint: String,
    pub signing_fingerprint: String,
    pub author: String,
    pub url: String,
    pub dependencies: Vec<BinaryReprDependency>,
}

impl BinaryReprMetadata {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            ident: String::new(),
            version,
            ident_key: String::new(),
            ident_fingerprint: String::new(),
            signing_fingerprint: String::new(),
            author: String::new(),
            url: String::new(),
            dependencies: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct BinaryReprDependency {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub pin: bool,
    pub flags: u32,
}

#[derive(Clone)]
pub struct BinaryReprExport {
    pub name: String,
    pub kind: BinaryReprExportKind,
    pub isolated: bool,
    pub params: Vec<BinaryReprExportParam>,
    pub return_type: String,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum BinaryReprExportKind {
    Func,
    Sub,
    Type,
    Union,
    Enum,
}

#[derive(Clone)]
pub struct BinaryReprExportParam {
    pub name: String,
    pub type_: String,
    pub has_default: bool,
}

#[derive(Clone)]
pub struct BinaryReprTypeExport {
    pub name: String,
    pub kind: BinaryReprExportKind,
    pub fields: Vec<BinaryReprTypeField>,
    pub variants: Vec<BinaryReprTypeVariant>,
    pub members: Vec<String>,
}

#[derive(Clone)]
pub struct BinaryReprTypeField {
    pub name: String,
    pub type_: String,
    pub visibility: BinaryReprTypeVisibility,
}

#[derive(Clone)]
pub struct BinaryReprTypeVariant {
    pub name: String,
    pub fields: Vec<BinaryReprTypeField>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum BinaryReprTypeVisibility {
    Private,
    Package,
    Export,
}

pub struct BinaryReprPackageInfo {
    pub manifest_name: String,
    pub manifest_ident: String,
    pub manifest_version: String,
    pub manifest_ident_key: String,
    pub manifest_ident_fingerprint: String,
    pub manifest_signing_fingerprint: String,
    pub author: String,
    pub url: String,
    pub type_count: usize,
    pub const_count: usize,
    pub resource_count: usize,
    pub function_count: usize,
    pub global_count: usize,
    pub export_count: usize,
    pub import_count: usize,
    pub cleanup_count: usize,
    pub abi_format_version: u16,
    pub exports: Vec<BinaryReprPackageInfoExport>,
    pub globals: Vec<BinaryReprPackageInfoGlobal>,
    pub imports: Vec<BinaryReprPackageInfoImport>,
    pub cleanups: Vec<BinaryReprPackageInfoCleanup>,
}

pub struct BinaryReprPackageInfoCleanup {
    pub function: String,
    pub cleanup_id: u32,
    pub start_pc: u32,
    pub end_pc: u32,
    pub resource_register: u32,
    pub close_function_id: u32,
    pub records_secondary_close_failure: bool,
}

pub struct BinaryReprPackageInfoGlobal {
    pub name: String,
    pub type_: String,
    pub mutable: bool,
    pub visibility: String,
}

pub struct BinaryReprPackageInfoExport {
    pub name: String,
    pub kind: BinaryReprExportKind,
    pub sig_hash: String,
}

pub struct BinaryReprPackageInfoImport {
    pub package_name: String,
    pub package_ident: String,
    pub version: String,
    pub pin: bool,
    pub flags: u32,
    pub used_symbols: Vec<BinaryReprPackageInfoUsedSymbol>,
}

pub struct BinaryReprPackageInfoUsedSymbol {
    pub name: String,
    pub sig_hash: String,
}

const RESOURCE_FLAG_NATIVE: u32 = 1 << 0;
const RESOURCE_FLAG_STANDARD: u32 = 1 << 1;
const RESOURCE_FLAG_SENDABLE: u32 = 1 << 2;
const RESOURCE_FLAG_CLOSE_MAY_FAIL: u32 = 1 << 3;
const CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE: u32 = 1 << 0;
pub(crate) const BUILTIN_FS_CLOSE_FUNCTION_ID: u32 = 0xffff_ff00;
pub(crate) const BUILTIN_NET_CLOSE_FUNCTION_ID: u32 = 0xffff_feff;

pub fn read_package_exports(path: &Path) -> Result<Vec<BinaryReprExport>, String> {
    let package = read_package_binary_repr(path)?;
    package_exports(&package).map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

pub fn read_package_info(path: &Path) -> Result<BinaryReprPackageInfo, String> {
    let package = read_package_binary_repr(path)?;
    package_info(&package).map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

pub fn read_package_type_exports(path: &Path) -> Result<Vec<BinaryReprTypeExport>, String> {
    let package = read_package_binary_repr(path)?;
    package_type_exports(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

/// One resource type contributed by an imported package's `RESOURCE_TABLE`.
//
// `native` distinguishes native (`LINK`) resources from standard ones; it is
// read by the later native-resource phase (`plan-link-update`).
#[allow(dead_code)]
pub struct BinaryReprResourceExport {
    pub type_name: String,
    /// Resolved close-op name (`fs.close`/`net.close` for built-ins, or the
    /// declaring package's close function name). `None` when the close function
    /// id cannot be resolved.
    pub close_function: Option<String>,
    pub sendable: bool,
    pub close_may_fail: bool,
    pub native: bool,
}

/// Decode an imported package's `RESOURCE_TABLE` so the importer can register
/// the package's resource types (recognition, sendability, and close op) instead
/// of relying on hardcoded knowledge of the standard built-ins.
pub fn read_package_resources(path: &Path) -> Result<Vec<BinaryReprResourceExport>, String> {
    let package = read_package_binary_repr(path)?;
    package_resource_exports(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

fn package_resource_exports(
    package: &PackageBinaryRepr,
) -> Result<Vec<BinaryReprResourceExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    let mut exports = Vec::with_capacity(package.project.resources.entries.len());
    for entry in &package.project.resources.entries {
        let type_name = type_name(&type_names, entry.type_id)?.to_string();
        // A native LINK resource (NATIVE set, STANDARD clear) stores its close op
        // name directly in the string pool (plan-link-update.md §10); built-ins
        // and source resources reference a function id / sentinel.
        let close_function = if entry.flags & RESOURCE_FLAG_NATIVE != 0
            && entry.flags & RESOURCE_FLAG_STANDARD == 0
        {
            Some(string_at(&package.project.strings.values, entry.close_function_id)?.to_string())
        } else {
            resolve_resource_close_name(package, entry.close_function_id)?
        };
        exports.push(BinaryReprResourceExport {
            type_name,
            close_function,
            sendable: entry.flags & RESOURCE_FLAG_SENDABLE != 0,
            close_may_fail: entry.flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0,
            native: entry.flags & RESOURCE_FLAG_NATIVE != 0,
        });
    }
    Ok(exports)
}

/// Resolve a `RESOURCE_TABLE` close-function id to a call name. The two built-in
/// sentinels map to the standard `fs.close`/`net.close` ops; any other id is a
/// `functionId` index into the package's function table.
fn resolve_resource_close_name(
    package: &PackageBinaryRepr,
    close_function_id: u32,
) -> Result<Option<String>, String> {
    match close_function_id {
        BUILTIN_FS_CLOSE_FUNCTION_ID => {
            Ok(builtins::resource_close_function(builtins::fs::FILE_TYPE).map(str::to_string))
        }
        BUILTIN_NET_CLOSE_FUNCTION_ID => {
            Ok(builtins::resource_close_function(builtins::net::SOCKET_TYPE).map(str::to_string))
        }
        id => match package.project.functions.get(id as usize) {
            Some(function) => Ok(Some(
                string_at(&package.project.strings.values, function.name)?.to_string(),
            )),
            None => Ok(None),
        },
    }
}

/// Decode a package's structured Binary Representation payload back into an `IrProject`.
///
/// This is the consumer entry point for the single `IR -> NIR -> native` path:
/// the returned IR is merged into the importing project and lowered like any
/// other function, replacing the old flat binary_repr -> native bridge.
pub fn read_package_ir_with_identity(
    path: &Path,
) -> Result<(String, crate::ir::IrProject), String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let container = mfp_binary_repr_payload(&bytes)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let package = read_binary_repr_package(container.binary_repr)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    validate_container_manifest_identity(&container.identity, &package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let id = package_identity_id(&container.identity, container.binary_repr);
    let ir = crate::ir::decode_binary_repr(&package.project.binary_repr)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    Ok((id, ir))
}

/// Deterministic per-package identity prefix segment (`<id>` in
/// `<id>.package.symbol`): a 16-hex-char content hash over the package's
/// manifest identity (name, version, ident) and its inner binary_repr payload.
///
/// Being a pure content hash, the same package always yields the same id —
/// giving reproducible builds and letting a diamond dependency de-duplicate to
/// a single copy — while differing content yields a differing id, keeping two
/// distinct packages (e.g. a version conflict) from colliding at merge time.
fn package_identity_id(identity: &MfpIdentity, payload: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    for field in [&identity.name, &identity.version, &identity.ident] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut id = String::with_capacity(16);
    for byte in &digest[..8] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}

fn read_package_binary_repr(path: &Path) -> Result<PackageBinaryRepr, String> {
    let package =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let container = mfp_binary_repr_payload(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let package = read_binary_repr_package(container.binary_repr)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    validate_container_manifest_identity(&container.identity, &package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    Ok(package)
}

struct MfpContainer<'a> {
    identity: MfpIdentity,
    binary_repr: &'a [u8],
}

struct MfpIdentity {
    name: String,
    ident: String,
    version: String,
    ident_key: String,
    ident_fingerprint: String,
    signing_fingerprint: String,
}

fn mfp_binary_repr_payload(bytes: &[u8]) -> Result<MfpContainer<'_>, String> {
    const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
    if bytes.len() < 26 {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }
    let container_major = checked_u16_at(bytes, 8)?;
    if container_major != 1 {
        return Err(format!(
            "unsupported MFP container major version {container_major}"
        ));
    }
    let signature_type = checked_u16_at(bytes, 20)?;
    let signature_length = checked_u32_at(bytes, 22)? as usize;
    validate_mfp_signature_header(signature_type, signature_length)?;
    let mut offset = 26usize
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if offset > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let name = read_length_prefixed(bytes, &mut offset, "name")?;
    let ident = read_length_prefixed(bytes, &mut offset, "ident")?;
    let version = read_length_prefixed(bytes, &mut offset, "version")?;
    let ident_key = read_length_prefixed(bytes, &mut offset, "identKey")?;
    let ident_fingerprint = read_length_prefixed(bytes, &mut offset, "identFingerprint")?;
    let signing_fingerprint = read_length_prefixed(bytes, &mut offset, "signingFingerprint")?;
    skip_length_prefixed(bytes, &mut offset, "author")?;
    skip_length_prefixed(bytes, &mut offset, "url")?;
    let binary_repr_length = checked_u64_at(bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    let end = offset
        .checked_add(binary_repr_length)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if end != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
    }
    Ok(MfpContainer {
        identity: MfpIdentity {
            name,
            ident,
            version,
            ident_key,
            ident_fingerprint,
            signing_fingerprint,
        },
        binary_repr: &bytes[offset..end],
    })
}

fn validate_mfp_signature_header(
    signature_type: u16,
    signature_length: usize,
) -> Result<(), String> {
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => Ok(()),
        (0, _) => Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => Err(format!("unsupported .mfp signature type {signature_type}")),
    }
}

fn validate_container_manifest_identity(
    identity: &MfpIdentity,
    package: &PackageBinaryRepr,
) -> Result<(), String> {
    let strings = &package.project.strings.values;
    let manifest = &package.project.manifest;
    let manifest_name = string_at(strings, manifest.package_name)?;
    let manifest_ident = string_at(strings, manifest.package_ident)?;
    let manifest_version = string_at(strings, manifest.package_version)?;
    let manifest_ident_key = string_at(strings, manifest.ident_key)?;
    let manifest_ident_fingerprint = string_at(strings, manifest.ident_fingerprint)?;
    let manifest_signing_fingerprint = string_at(strings, manifest.signing_fingerprint)?;
    if identity.name != manifest_name
        || identity.ident != manifest_ident
        || identity.version != manifest_version
        || identity.ident_key != manifest_ident_key
        || identity.ident_fingerprint != manifest_ident_fingerprint
        || identity.signing_fingerprint != manifest_signing_fingerprint
    {
        return Err(
            "MFP header identity does not match binary representation manifest identity"
                .to_string(),
        );
    }
    Ok(())
}

fn read_binary_repr_package(bytes: &[u8]) -> Result<PackageBinaryRepr, String> {
    if bytes.len() < 16 || &bytes[0..4] != b"MFPC" {
        return Err(
            "package payload does not have the binary representation container magic".to_string(),
        );
    }
    let major = checked_u16_at(bytes, 4)?;
    if major != MFPC_MAJOR_VERSION {
        return Err(format!(
            "unsupported MFPC major version {major} (expected {MFPC_MAJOR_VERSION}); \
             this package predates the structured Binary Representation format and must be rebuilt"
        ));
    }
    let section_count = checked_u32_at(bytes, 12)? as usize;
    let table_end = 16usize
        .checked_add(
            section_count
                .checked_mul(24)
                .ok_or_else(|| "invalid MFPC section table length".to_string())?,
        )
        .ok_or_else(|| "invalid MFPC section table length".to_string())?;
    if table_end > bytes.len() {
        return Err("truncated MFPC section table".to_string());
    }

    let mut sections = HashMap::new();
    for index in 0..section_count {
        let entry = 16 + index * 24;
        let id = checked_u16_at(bytes, entry)?;
        let offset = checked_u64_at(bytes, entry + 8)? as usize;
        let length = checked_u64_at(bytes, entry + 16)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid MFPC section length".to_string())?;
        if end > bytes.len() {
            return Err("truncated MFPC section".to_string());
        }
        sections.insert(id, &bytes[offset..end]);
    }

    let string_values = read_string_pool(
        sections
            .get(&SECTION_STRING_POOL)
            .copied()
            .ok_or_else(|| "MFPC is missing the string pool section".to_string())?,
    )?;
    let strings = StringPool {
        values: string_values,
    };
    let types = read_type_entries(
        sections
            .get(&SECTION_TYPE_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the type table section".to_string())?,
        &strings.values,
    )?;
    let type_names = type_entry_names(&types, &strings.values)?;
    let constants = read_const_pool(
        sections
            .get(&SECTION_CONST_POOL)
            .copied()
            .ok_or_else(|| "MFPC is missing the const pool section".to_string())?,
    )?;
    let functions = read_function_table(
        sections
            .get(&SECTION_FUNCTION_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the function table section".to_string())?,
        // Function bodies are carried by SECTION_BINARY_REPR (structured Binary Representation), not a
        // flat code section; the function table records zero-length code regions.
        &[],
        &strings.values,
        &type_names,
    )?;
    let binary_repr = sections
        .get(&SECTION_BINARY_REPR)
        .copied()
        .ok_or_else(|| "MFPC is missing the Binary Representation section".to_string())?
        .to_vec();
    let exports = read_export_table(
        sections
            .get(&SECTION_EXPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the export table section".to_string())?,
    )?;
    let resources = match sections.get(&SECTION_RESOURCE_TABLE).copied() {
        Some(section) => read_resource_table(section)?,
        None => ResourceTable::new(),
    };
    let globals = match sections.get(&SECTION_GLOBAL_TABLE).copied() {
        Some(section) => read_global_table(section)?,
        None => Vec::new(),
    };
    let manifest = read_manifest(
        sections
            .get(&SECTION_MANIFEST)
            .copied()
            .ok_or_else(|| "MFPC is missing the manifest section".to_string())?,
    )?;
    let imports = read_import_table(
        sections
            .get(&SECTION_IMPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFPC is missing the import table section".to_string())?,
    )?;
    let abi = read_abi_index(
        sections
            .get(&SECTION_ABI_INDEX)
            .copied()
            .ok_or_else(|| "MFPC is missing the ABI_INDEX section".to_string())?,
    )?;
    validate_abi_index(
        &abi,
        &exports,
        &imports,
        &strings.values,
        &types,
        &constants,
        &functions,
    )?;

    Ok(PackageBinaryRepr {
        project: BinaryReprProject {
            strings,
            types,
            constants,
            resources,
            globals,
            manifest,
            imports,
            abi,
            entry_function: u32::MAX,
            entry_flags: 0,
            functions,
            binary_repr,
        },
        exports,
    })
}

fn package_exports(package: &PackageBinaryRepr) -> Result<Vec<BinaryReprExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    package
        .exports
        .iter()
        .map(|export| {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            Ok(BinaryReprExport {
                name: string_at(&package.project.strings.values, export.name)?.to_string(),
                kind: export.kind,
                isolated: function.flags & FUNCTION_FLAG_ISOLATED != 0,
                params: function
                    .params
                    .iter()
                    .map(|param| {
                        Ok::<BinaryReprExportParam, String>(BinaryReprExportParam {
                            name: string_at(&package.project.strings.values, param.name)?
                                .to_string(),
                            type_: type_name(&type_names, param.type_id)?.to_string(),
                            has_default: param.flags & 1 != 0,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                return_type: type_name(&type_names, function.return_type)?.to_string(),
            })
        })
        .collect()
}

fn package_info(package: &PackageBinaryRepr) -> Result<BinaryReprPackageInfo, String> {
    let strings = &package.project.strings.values;
    let type_names = type_entry_names(&package.project.types, strings)?;
    let exports = package
        .project
        .abi
        .exports
        .iter()
        .map(|abi_export| {
            Ok(BinaryReprPackageInfoExport {
                name: string_at(strings, abi_export.name)?.to_string(),
                kind: abi_export.kind,
                sig_hash: hex_hash(&abi_export.sig_hash),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let globals = package
        .project
        .globals
        .iter()
        .map(|global| {
            let visibility = match (global.flags >> 1) & 0b11 {
                1 => "package",
                2 => "export",
                _ => "private",
            };
            Ok(BinaryReprPackageInfoGlobal {
                name: string_at(strings, global.name)?.to_string(),
                type_: type_name(&type_names, global.type_id)?.to_string(),
                mutable: global.flags & 1 != 0,
                visibility: visibility.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut abi_edges = package
        .project
        .abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok((
                (
                    string_at(strings, edge.package_name)?.to_string(),
                    string_at(strings, edge.package_ident)?.to_string(),
                ),
                edge,
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    let imports = package
        .project
        .imports
        .entries
        .iter()
        .map(|entry| {
            let package_name = string_at(strings, entry.package_name)?.to_string();
            let package_ident = string_at(strings, entry.package_ident)?.to_string();
            let edge = abi_edges.remove(&(package_name.clone(), package_ident.clone()));
            let used_symbols = edge
                .map(|edge| {
                    edge.used_symbols
                        .iter()
                        .map(|symbol| {
                            Ok(BinaryReprPackageInfoUsedSymbol {
                                name: string_at(strings, symbol.name)?.to_string(),
                                sig_hash: hex_hash(&symbol.sig_hash),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(BinaryReprPackageInfoImport {
                package_name,
                package_ident,
                version: string_at(strings, entry.version)?.to_string(),
                pin: entry.pin,
                flags: entry.flags,
                used_symbols,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let cleanups = package
        .project
        .functions
        .iter()
        .flat_map(|function| {
            function
                .cleanups
                .iter()
                .map(move |cleanup| (function.name, cleanup))
        })
        .map(|(function_name, cleanup)| {
            Ok(BinaryReprPackageInfoCleanup {
                function: string_at(strings, function_name)?.to_string(),
                cleanup_id: cleanup.id,
                start_pc: cleanup.start_pc,
                end_pc: cleanup.end_pc,
                resource_register: cleanup.resource_register,
                close_function_id: cleanup.close_function_id,
                records_secondary_close_failure: cleanup.flags
                    & CLEANUP_FLAG_RECORD_SECONDARY_CLOSE_FAILURE
                    != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(BinaryReprPackageInfo {
        manifest_name: string_at(strings, package.project.manifest.package_name)?.to_string(),
        manifest_ident: string_at(strings, package.project.manifest.package_ident)?.to_string(),
        manifest_version: string_at(strings, package.project.manifest.package_version)?.to_string(),
        manifest_ident_key: string_at(strings, package.project.manifest.ident_key)?.to_string(),
        manifest_ident_fingerprint: string_at(strings, package.project.manifest.ident_fingerprint)?
            .to_string(),
        manifest_signing_fingerprint: string_at(
            strings,
            package.project.manifest.signing_fingerprint,
        )?
        .to_string(),
        author: string_at(strings, package.project.manifest.author)?.to_string(),
        url: string_at(strings, package.project.manifest.url)?.to_string(),
        type_count: package.project.types.entries.len(),
        const_count: package.project.constants.entries.len(),
        resource_count: package.project.resources.entries.len(),
        function_count: package.project.functions.len(),
        global_count: package.project.globals.len(),
        export_count: package.project.abi.exports.len(),
        import_count: package.project.imports.entries.len(),
        cleanup_count: cleanups.len(),
        abi_format_version: ABI_FORMAT_VERSION,
        exports,
        globals,
        imports,
        cleanups,
    })
}

fn package_type_exports(package: &PackageBinaryRepr) -> Result<Vec<BinaryReprTypeExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    let type_by_name = package
        .project
        .types
        .entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let id = FIRST_TABLE_TYPE_ID + index as u32;
            type_name(&type_names, id)
                .ok()
                .map(|name| (name.to_string(), entry))
        })
        .collect::<HashMap<_, _>>();
    let mut exports = Vec::new();
    for export in &package.project.abi.exports {
        if !matches!(
            export.kind,
            BinaryReprExportKind::Type | BinaryReprExportKind::Union | BinaryReprExportKind::Enum
        ) {
            continue;
        }
        let name = string_at(&package.project.strings.values, export.name)?.to_string();
        let Some(entry) = type_by_name.get(&name) else {
            return Err(format!(
                "exported type `{name}` is missing from the type table"
            ));
        };
        exports.push(decode_type_export(
            &name,
            export.kind,
            entry,
            &type_names,
            &package.project.strings.values,
        )?);
    }
    Ok(exports)
}

fn decode_type_export(
    name: &str,
    kind: BinaryReprExportKind,
    entry: &TypeEntry,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeExport, String> {
    let mut offset = 0usize;
    let (fields, variants, members) = match kind {
        BinaryReprExportKind::Type => {
            let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut fields = Vec::with_capacity(field_count);
            for _ in 0..field_count {
                fields.push(decode_type_field(
                    &entry.payload,
                    &mut offset,
                    type_names,
                    strings,
                )?);
            }
            (fields, Vec::new(), Vec::new())
        }
        BinaryReprExportKind::Union => {
            let variant_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut variants = Vec::with_capacity(variant_count);
            for _ in 0..variant_count {
                let variant_name =
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
                let mut fields = Vec::with_capacity(field_count);
                for _ in 0..field_count {
                    let field_name =
                        string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string();
                    let field_type =
                        type_name(type_names, cursor_u32(&entry.payload, &mut offset)?)?
                            .to_string();
                    fields.push(BinaryReprTypeField {
                        name: field_name,
                        type_: field_type,
                        visibility: BinaryReprTypeVisibility::Export,
                    });
                }
                variants.push(BinaryReprTypeVariant {
                    name: variant_name,
                    fields,
                });
            }
            (Vec::new(), variants, Vec::new())
        }
        BinaryReprExportKind::Enum => {
            let member_count = cursor_u32(&entry.payload, &mut offset)? as usize;
            let mut members = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                members.push(
                    string_at(strings, cursor_u32(&entry.payload, &mut offset)?)?.to_string(),
                );
                let _ordinal = cursor_u32(&entry.payload, &mut offset)?;
            }
            (Vec::new(), Vec::new(), members)
        }
        BinaryReprExportKind::Func | BinaryReprExportKind::Sub => {
            return Err(format!("export `{name}` is not a type export"));
        }
    };
    if offset != entry.payload.len() {
        return Err(format!("exported type `{name}` has trailing payload bytes"));
    }
    Ok(BinaryReprTypeExport {
        name: name.to_string(),
        kind,
        fields,
        variants,
        members,
    })
}

fn decode_type_field(
    payload: &[u8],
    offset: &mut usize,
    type_names: &HashMap<u32, String>,
    strings: &[String],
) -> Result<BinaryReprTypeField, String> {
    let name = string_at(strings, cursor_u32(payload, offset)?)?.to_string();
    let type_ = type_name(type_names, cursor_u32(payload, offset)?)?.to_string();
    let visibility = match cursor_u32(payload, offset)? {
        0 => BinaryReprTypeVisibility::Export,
        1 => BinaryReprTypeVisibility::Private,
        2 => BinaryReprTypeVisibility::Package,
        3 => BinaryReprTypeVisibility::Export,
        other => return Err(format!("unsupported type field visibility {other}")),
    };
    Ok(BinaryReprTypeField {
        name,
        type_,
        visibility,
    })
}

struct DecodedExport {
    name: u32,
    kind: BinaryReprExportKind,
    function_id: u32,
}

fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut strings = Vec::with_capacity(count);
    for _ in 0..count {
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid string pool entry length".to_string())?;
        if end > bytes.len() {
            return Err("truncated string pool entry".to_string());
        }
        strings.push(
            std::str::from_utf8(&bytes[offset..end])
                .map_err(|_| "string pool entry is not valid UTF-8".to_string())?
                .to_string(),
        );
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in string pool".to_string());
    }
    Ok(strings)
}

fn read_type_entries(bytes: &[u8], strings: &[String]) -> Result<TypeTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let entries_end = 4usize
        .checked_add(
            count
                .checked_mul(20)
                .ok_or_else(|| "invalid type table length".to_string())?,
        )
        .ok_or_else(|| "invalid type table length".to_string())?;
    if entries_end > bytes.len() {
        return Err("truncated type table".to_string());
    }

    let mut entries = Vec::with_capacity(count);
    let mut ids = HashMap::new();
    for index in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let name = cursor_u32(bytes, &mut offset)?;
        let owner_package = cursor_u32(bytes, &mut offset)?;
        let payload_offset = cursor_u32(bytes, &mut offset)? as usize;
        let payload_length = cursor_u32(bytes, &mut offset)? as usize;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| "invalid type payload length".to_string())?;
        if payload_offset < entries_end || payload_end > bytes.len() {
            return Err("invalid type payload bounds".to_string());
        }
        let id = FIRST_TABLE_TYPE_ID + index as u32;
        ids.insert(string_at(strings, name)?.to_string(), id);
        entries.push(TypeEntry {
            kind,
            name,
            owner_package,
            abi_export_kind: None,
            payload: bytes[payload_offset..payload_end].to_vec(),
        });
    }

    Ok(TypeTable { entries, ids })
}

fn type_entry_names(types: &TypeTable, strings: &[String]) -> Result<HashMap<u32, String>, String> {
    let raw = types
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                FIRST_TABLE_TYPE_ID + index as u32,
                (entry.kind, entry.name, entry.payload.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut decoded = HashMap::new();
    for id in raw.keys().copied().collect::<Vec<_>>() {
        let name = decode_type_name(id, &raw, strings, &mut decoded)?;
        decoded.insert(id, name);
    }
    Ok(decoded)
}

fn decode_type_name(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name.to_string());
    }
    if let Some(name) = decoded.get(&id) {
        return Ok(name.clone());
    }
    let Some((kind, name, payload)) = raw.get(&id) else {
        return Err(format!("unknown type id {id}"));
    };
    let decoded_name = match *kind {
        4 => {
            let element = read_payload_type(payload, 0, raw, strings, decoded)?;
            format!("List OF {element}")
        }
        5 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded)?;
            format!("Map OF {key} TO {value}")
        }
        6 => {
            let success = read_payload_type(payload, 0, raw, strings, decoded)?;
            format!("Result OF {success}")
        }
        7 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(payload, 8, raw, strings, decoded)?)
            } else {
                None
            };
            builtins::thread::format_thread_type("Thread", &message, resource.as_deref(), &output)
        }
        8 => decode_function_type(payload, raw, strings, decoded)?,
        9 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded)?;
            format!("MapEntry OF {key} TO {value}")
        }
        10 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded)?;
            let resource = if payload.len() >= 12 {
                Some(read_payload_type(payload, 8, raw, strings, decoded)?)
            } else {
                None
            };
            builtins::thread::format_thread_type(
                "ThreadWorker",
                &message,
                resource.as_deref(),
                &output,
            )
        }
        _ => string_at(strings, *name)?.to_string(),
    };
    decoded.insert(id, decoded_name.clone());
    Ok(decoded_name)
}

fn decode_function_type(
    payload: &[u8],
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    let mut offset = 0;
    let isolated = cursor_u32(payload, &mut offset)? != 0;
    let param_count = cursor_u32(payload, &mut offset)? as usize;
    let return_type = cursor_u32(payload, &mut offset)?;
    let returns = decode_type_name(return_type, raw, strings, decoded)?;
    let mut params = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        let param = cursor_u32(payload, &mut offset)?;
        params.push(decode_type_name(param, raw, strings, decoded)?);
    }
    let prefix = if isolated { "ISOLATED FUNC" } else { "FUNC" };
    Ok(format!("{prefix}({}) AS {returns}", params.join(", ")))
}

fn read_payload_type(
    payload: &[u8],
    offset: usize,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    let id = checked_u32_at(payload, offset)?;
    decode_type_name(id, raw, strings, decoded)
}

fn primitive_type_name(id: u32) -> Option<&'static str> {
    match id {
        TYPE_NOTHING => Some("Nothing"),
        TYPE_BOOLEAN => Some("Boolean"),
        TYPE_INTEGER => Some("Integer"),
        TYPE_FLOAT => Some("Float"),
        TYPE_FIXED => Some("Fixed"),
        TYPE_STRING => Some("String"),
        TYPE_BYTE => Some("Byte"),
        TYPE_ERROR => Some("Error"),
        TYPE_TERMINAL_SIZE => Some("TerminalSize"),
        TYPE_FILE_HANDLE => Some("File"),
        TYPE_SOCKET_HANDLE => Some("Socket"),
        TYPE_LISTENER_HANDLE => Some("Listener"),
        _ => None,
    }
}

fn read_function_table(
    bytes: &[u8],
    code: &[u8],
    strings: &[String],
    _types: &HashMap<u32, String>,
) -> Result<Vec<Function>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut functions = Vec::with_capacity(count);
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = cursor_u16(bytes, &mut offset)?;
        let flags = cursor_u16(bytes, &mut offset)?;
        let param_count = cursor_u32(bytes, &mut offset)? as usize;
        let return_type = cursor_u32(bytes, &mut offset)?;
        let register_count = cursor_u32(bytes, &mut offset)? as usize;
        let code_offset = cursor_u64(bytes, &mut offset)? as usize;
        let code_length = cursor_u64(bytes, &mut offset)? as usize;
        let _source_map = cursor_u32(bytes, &mut offset)?;
        let cleanup_count = cursor_u32(bytes, &mut offset)? as usize;
        let _cleanup_offset = cursor_u64(bytes, &mut offset)?;

        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            let param_name = cursor_u32(bytes, &mut offset)?;
            let _ = string_at(strings, param_name)?;
            let param_type = cursor_u32(bytes, &mut offset)?;
            let param_flags = cursor_u32(bytes, &mut offset)?;
            let default_const = cursor_u32(bytes, &mut offset)?;
            params.push(Param {
                name: param_name,
                type_id: param_type,
                flags: param_flags,
                default_const,
            });
        }
        let mut registers = Vec::with_capacity(register_count);
        for _ in 0..register_count {
            registers.push(Register {
                type_id: cursor_u32(bytes, &mut offset)?,
                flags: cursor_u32(bytes, &mut offset)?,
            });
        }
        let mut cleanups = Vec::with_capacity(cleanup_count);
        for _ in 0..cleanup_count {
            cleanups.push(Cleanup {
                id: cursor_u32(bytes, &mut offset)?,
                start_pc: cursor_u32(bytes, &mut offset)?,
                end_pc: cursor_u32(bytes, &mut offset)?,
                resource_register: cursor_u32(bytes, &mut offset)?,
                close_function_id: cursor_u32(bytes, &mut offset)?,
                flags: cursor_u32(bytes, &mut offset)?,
            });
        }

        let code_end = code_offset
            .checked_add(code_length)
            .ok_or_else(|| "invalid function code length".to_string())?;
        if code_end > code.len() {
            return Err("truncated function code".to_string());
        }
        // Function bodies live in SECTION_BINARY_REPR (structured Binary Representation), so the flat
        // code region is always empty here.
        if code_length != 0 {
            return Err("flat function code stream is no longer supported".to_string());
        }
        functions.push(Function {
            name,
            kind,
            flags,
            return_type,
            params,
            registers,
            cleanups,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in function table".to_string());
    }
    Ok(functions)
}

fn read_const_pool(bytes: &[u8]) -> Result<ConstPool, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid const payload length".to_string())?;
        if end > bytes.len() {
            return Err("truncated const payload".to_string());
        }
        entries.push(ConstEntry {
            kind,
            payload: bytes[offset..end].to_vec(),
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in const pool".to_string());
    }
    Ok(ConstPool { entries })
}

fn read_manifest(bytes: &[u8]) -> Result<BinaryReprManifest, String> {
    let mut offset = 0;
    let manifest = BinaryReprManifest {
        package_name: cursor_u32(bytes, &mut offset)?,
        package_ident: cursor_u32(bytes, &mut offset)?,
        package_version: cursor_u32(bytes, &mut offset)?,
        ident_key: cursor_u32(bytes, &mut offset)?,
        ident_fingerprint: cursor_u32(bytes, &mut offset)?,
        signing_fingerprint: cursor_u32(bytes, &mut offset)?,
        author: cursor_u32(bytes, &mut offset)?,
        url: cursor_u32(bytes, &mut offset)?,
    };
    let _binary_repr_major = cursor_u16(bytes, &mut offset)?;
    let _binary_repr_minor = cursor_u16(bytes, &mut offset)?;
    let _language_major = cursor_u16(bytes, &mut offset)?;
    let _language_minor = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_major = cursor_u16(bytes, &mut offset)?;
    let _minimum_runtime_minor = cursor_u16(bytes, &mut offset)?;
    let _dependency_count = cursor_u32(bytes, &mut offset)?;
    let _native_link_count = cursor_u32(bytes, &mut offset)?;
    let _export_count = cursor_u32(bytes, &mut offset)?;
    let _entry_function = cursor_u32(bytes, &mut offset)?;
    let _entry_flags = cursor_u32(bytes, &mut offset)?;
    if offset != bytes.len() {
        return Err("invalid trailing bytes in manifest".to_string());
    }
    Ok(manifest)
}

fn read_import_table(bytes: &[u8]) -> Result<ImportTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ImportEntry {
            package_name: cursor_u32(bytes, &mut offset)?,
            package_ident: cursor_u32(bytes, &mut offset)?,
            version: cursor_u32(bytes, &mut offset)?,
            pin: match cursor_u8(bytes, &mut offset)? {
                0 => false,
                1 => true,
                other => return Err(format!("unsupported import pin value {other}")),
            },
            flags: cursor_u32(bytes, &mut offset)?,
            used_symbols: read_used_symbols(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in import table".to_string());
    }
    Ok(ImportTable { entries })
}

fn read_used_symbols(bytes: &[u8], offset: &mut usize) -> Result<Vec<AbiUsedSymbol>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut symbols = Vec::with_capacity(count);
    for _ in 0..count {
        symbols.push(AbiUsedSymbol {
            name: cursor_u32(bytes, offset)?,
            sig_hash: cursor_hash(bytes, offset)?,
        });
    }
    Ok(symbols)
}

fn read_resource_table(bytes: &[u8]) -> Result<ResourceTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ResourceEntry {
            type_id: cursor_u32(bytes, &mut offset)?,
            close_function_id: cursor_u32(bytes, &mut offset)?,
            flags: cursor_u32(bytes, &mut offset)?,
        });
    }
    Ok(ResourceTable { entries })
}

fn read_global_table(bytes: &[u8]) -> Result<Vec<GlobalEntry>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut globals = Vec::with_capacity(count);
    for _ in 0..count {
        globals.push(GlobalEntry {
            name: cursor_u32(bytes, &mut offset)?,
            type_id: cursor_u32(bytes, &mut offset)?,
            flags: cursor_u32(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in global table".to_string());
    }
    Ok(globals)
}

fn read_export_table(bytes: &[u8]) -> Result<Vec<DecodedExport>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(count);
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            kind => decode_callable_export_kind(kind)?,
        };
        let _flags = cursor_u16(bytes, &mut offset)?;
        let function_id = cursor_u32(bytes, &mut offset)?;
        exports.push(DecodedExport {
            name,
            kind,
            function_id,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in export table".to_string());
    }
    Ok(exports)
}

fn read_abi_index(bytes: &[u8]) -> Result<AbiIndex, String> {
    let mut offset = 0;
    let version = cursor_u16(bytes, &mut offset)?;
    if version != ABI_FORMAT_VERSION {
        return Err(format!("unsupported ABI_INDEX format version {version}"));
    }
    let _reserved = cursor_u16(bytes, &mut offset)?;

    let export_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(export_count);
    for _ in 0..export_count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            kind => decode_export_kind(kind)?,
        };
        let sig_hash = cursor_hash(bytes, &mut offset)?;
        exports.push(AbiExport {
            name,
            kind,
            sig_hash,
        });
    }

    let edge_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut dep_edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        let package_name = cursor_u32(bytes, &mut offset)?;
        let package_ident = cursor_u32(bytes, &mut offset)?;
        let version_request = cursor_u32(bytes, &mut offset)?;
        let pin = match cursor_u8(bytes, &mut offset)? {
            0 => false,
            1 => true,
            other => return Err(format!("unsupported ABI_INDEX dep pin value {other}")),
        };
        let used_count = cursor_u32(bytes, &mut offset)? as usize;
        let mut used_symbols = Vec::with_capacity(used_count);
        for _ in 0..used_count {
            used_symbols.push(AbiUsedSymbol {
                name: cursor_u32(bytes, &mut offset)?,
                sig_hash: cursor_hash(bytes, &mut offset)?,
            });
        }
        dep_edges.push(AbiDepEdge {
            package_name,
            package_ident,
            version_request,
            pin,
            used_symbols,
        });
    }

    if offset != bytes.len() {
        return Err("invalid trailing bytes in ABI_INDEX".to_string());
    }

    Ok(AbiIndex { exports, dep_edges })
}

fn validate_abi_index(
    abi: &AbiIndex,
    exports: &[DecodedExport],
    imports: &ImportTable,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
    functions: &[Function],
) -> Result<(), String> {
    for export in exports {
        let name = string_at(strings, export.name).unwrap_or("<invalid>");
        let Some(abi_export) = abi_export_for_decoded(abi, export) else {
            return Err(format!("ABI_INDEX is missing EXPORT_TABLE entry `{name}`"));
        };
        let Some(function) = functions.get(export.function_id as usize) else {
            return Err(format!(
                "export references missing function {}",
                export.function_id
            ));
        };
        let expected = function_sig_hash(function, export.kind, strings, types, constants)?;
        if abi_export.sig_hash != expected {
            return Err(format!(
                "ABI_INDEX export `{name}` sigHash disagrees with binary representation (required {}, provided {})",
                hex_hash(&expected),
                hex_hash(&abi_export.sig_hash)
            ));
        }
    }

    let import_names = imports
        .entries
        .iter()
        .map(|entry| {
            Ok::<(String, String), String>((
                string_at(strings, entry.package_name)?.to_string(),
                string_at(strings, entry.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let edge_names = abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok::<(String, String), String>((
                string_at(strings, edge.package_name)?.to_string(),
                string_at(strings, edge.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sorted_pairs(import_names) != sorted_pairs(edge_names) {
        return Err("ABI_INDEX dependency edges disagree with IMPORT_TABLE entries".to_string());
    }

    for import in &imports.entries {
        let Some(edge) = abi.dep_edges.iter().find(|edge| {
            edge.package_name == import.package_name && edge.package_ident == import.package_ident
        }) else {
            continue;
        };
        if edge.version_request != import.version || edge.pin != import.pin {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE request"
            ));
        }
        if edge.used_symbols.len() != import.used_symbols.len()
            || edge
                .used_symbols
                .iter()
                .zip(import.used_symbols.iter())
                .any(|(a, b)| a.name != b.name || a.sig_hash != b.sig_hash)
        {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE used symbols"
            ));
        }
    }

    Ok(())
}

fn abi_export_for_decoded<'a>(abi: &'a AbiIndex, export: &DecodedExport) -> Option<&'a AbiExport> {
    abi.exports
        .iter()
        .find(|abi_export| abi_export.name == export.name && abi_export.kind == export.kind)
}

fn decode_callable_export_kind(value: u16) -> Result<BinaryReprExportKind, String> {
    match decode_export_kind(value)? {
        BinaryReprExportKind::Func => Ok(BinaryReprExportKind::Func),
        BinaryReprExportKind::Sub => Ok(BinaryReprExportKind::Sub),
        other => Err(format!(
            "unsupported callable export kind {}",
            encode_export_kind(other)
        )),
    }
}

fn decode_export_kind(value: u16) -> Result<BinaryReprExportKind, String> {
    match value {
        1 => Ok(BinaryReprExportKind::Func),
        2 => Ok(BinaryReprExportKind::Sub),
        3 => Ok(BinaryReprExportKind::Type),
        4 => Ok(BinaryReprExportKind::Union),
        5 => Ok(BinaryReprExportKind::Enum),
        other => Err(format!("unsupported export kind {other}")),
    }
}

fn encode_export_kind(kind: BinaryReprExportKind) -> u16 {
    match kind {
        BinaryReprExportKind::Func => 1,
        BinaryReprExportKind::Sub => 2,
        BinaryReprExportKind::Type => 3,
        BinaryReprExportKind::Union => 4,
        BinaryReprExportKind::Enum => 5,
    }
}

fn type_name(types: &HashMap<u32, String>, id: u32) -> Result<&str, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name);
    }
    types
        .get(&id)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown type id {id}"))
}

fn string_at(strings: &[String], id: u32) -> Result<&str, String> {
    strings
        .get(id as usize)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown string id {id}"))
}

fn function_sig_hash(
    function: &Function,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("function");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.put_u16(function.flags & (FUNCTION_FLAG_ISOLATED | FUNCTION_FLAG_SUB));
    serializer.put_u32(function.params.len() as u32);
    for param in &function.params {
        serializer.serialize_type(param.type_id)?;
        if param.default_const == u32::MAX {
            serializer.put_u8(0);
        } else {
            serializer.put_u8(1);
            serializer.serialize_const(param.default_const)?;
        }
    }
    serializer.serialize_type(function.return_type)?;
    Ok(hash_bytes(&serializer.bytes))
}

fn type_sig_hash(
    type_id: u32,
    export_kind: BinaryReprExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("type");
    serializer.put_u16(encode_export_kind(export_kind));
    serializer.serialize_type(type_id)?;
    Ok(hash_bytes(&serializer.bytes))
}

struct AbiSerializer<'a> {
    strings: &'a [String],
    types: &'a TypeTable,
    constants: &'a ConstPool,
    bytes: Vec<u8>,
    type_refs: HashMap<u32, u32>,
    next_ref: u32,
}

impl<'a> AbiSerializer<'a> {
    fn new(strings: &'a [String], types: &'a TypeTable, constants: &'a ConstPool) -> Self {
        Self {
            strings,
            types,
            constants,
            bytes: Vec::new(),
            type_refs: HashMap::new(),
            next_ref: 0,
        }
    }

    fn serialize_type(&mut self, id: u32) -> Result<(), String> {
        if let Some(primitive) = primitive_type_name(id) {
            self.put_u8(1);
            self.put_u32(id);
            self.put_str(primitive);
            return Ok(());
        }

        if let Some(ref_id) = self.type_refs.get(&id).copied() {
            self.put_u8(2);
            self.put_u32(ref_id);
            return Ok(());
        }

        let entry = self
            .types
            .entries
            .get((id - FIRST_TABLE_TYPE_ID) as usize)
            .ok_or_else(|| format!("unknown type id {id}"))?;
        let ref_id = self.next_ref;
        self.next_ref = self
            .next_ref
            .checked_add(1)
            .ok_or_else(|| "ABI type graph has too many nodes".to_string())?;
        self.type_refs.insert(id, ref_id);

        self.put_u8(3);
        self.put_u32(ref_id);
        self.put_u16(entry.kind);
        match entry.kind {
            1 => self.serialize_record_type(entry),
            2 => self.serialize_union_type(entry),
            3 => self.serialize_enum_type(entry),
            4 => {
                self.put_str("list");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            5 => {
                self.put_str("map");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            6 => {
                self.put_str("result");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            7 => {
                self.put_str("thread");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)?;
                // The resource plane (if present) is part of the signature hash.
                if entry.payload.len() >= 12 {
                    self.serialize_type(checked_u32_at(&entry.payload, 8)?)?;
                }
                Ok(())
            }
            8 => self.serialize_function_type(entry),
            _ => {
                self.put_str("opaque");
                self.put_str(string_at(self.strings, entry.name)?);
                Ok(())
            }
        }
    }

    fn serialize_record_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("record");
        let mut offset = 0;
        let field_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(field_count);
        for _ in 0..field_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let type_id = cursor_u32(&entry.payload, &mut offset)?;
            let _visibility = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.serialize_type(type_id)?;
            self.put_u32(_visibility);
        }
        Ok(())
    }

    fn serialize_union_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("union");
        let mut offset = 0;
        let variant_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(variant_count);
        for _ in 0..variant_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            let field_count = cursor_u32(&entry.payload, &mut offset)?;
            self.put_u32(field_count);
            for _ in 0..field_count {
                let field_name = cursor_u32(&entry.payload, &mut offset)?;
                let field_type = cursor_u32(&entry.payload, &mut offset)?;
                self.put_str(string_at(self.strings, field_name)?);
                self.serialize_type(field_type)?;
            }
        }
        Ok(())
    }

    fn serialize_enum_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("enum");
        let mut offset = 0;
        let member_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(member_count);
        for _ in 0..member_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let ordinal = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.put_u32(ordinal);
        }
        Ok(())
    }

    fn serialize_function_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("function-type");
        let mut offset = 0;
        let isolated = cursor_u32(&entry.payload, &mut offset)?;
        let param_count = cursor_u32(&entry.payload, &mut offset)?;
        let return_type = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(isolated);
        self.put_u32(param_count);
        self.serialize_type(return_type)?;
        for _ in 0..param_count {
            self.serialize_type(cursor_u32(&entry.payload, &mut offset)?)?;
        }
        Ok(())
    }

    fn serialize_const(&mut self, id: u32) -> Result<(), String> {
        let constant = self
            .constants
            .entries
            .get(id as usize)
            .ok_or_else(|| format!("unknown const id {id}"))?;
        self.put_u16(constant.kind);
        match constant.kind {
            6 => {
                let string_id = checked_u32_at(&constant.payload, 0)?;
                self.put_str(string_at(self.strings, string_id)?);
            }
            _ => {
                self.put_u32(constant.payload.len() as u32);
                self.bytes.extend_from_slice(&constant.payload);
            }
        }
        Ok(())
    }

    fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn put_u16(&mut self, value: u16) {
        put_u16(&mut self.bytes, value);
    }

    fn put_u32(&mut self, value: u32) {
        put_u32(&mut self.bytes, value);
    }

    fn put_str(&mut self, value: &str) {
        put_bytes(&mut self.bytes, value.as_bytes());
    }
}

fn hash_bytes(bytes: &[u8]) -> [u8; ABI_HASH_LEN] {
    let digest = Sha256::digest(bytes);
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(&digest);
    hash
}

fn sorted_pairs(mut values: Vec<(String, String)>) -> Vec<(String, String)> {
    values.sort();
    values
}

fn hex_hash(hash: &[u8; ABI_HASH_LEN]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn skip_length_prefixed(bytes: &[u8], offset: &mut usize, field: &str) -> Result<(), String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }
    *offset = end;
    Ok(())
}

fn read_length_prefixed(bytes: &[u8], offset: &mut usize, field: &str) -> Result<String, String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| format!("truncated .mfp {field}"))?;
    *offset = end;
    String::from_utf8(value.to_vec()).map_err(|_| format!(".mfp {field} is not valid UTF-8"))
}

fn cursor_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, String> {
    let value = *bytes
        .get(*offset)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    *offset = offset
        .checked_add(1)
        .ok_or_else(|| "invalid u8 offset".to_string())?;
    Ok(value)
}

fn cursor_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, String> {
    let value = checked_u16_at(bytes, *offset)?;
    *offset = offset
        .checked_add(2)
        .ok_or_else(|| "invalid u16 offset".to_string())?;
    Ok(value)
}

fn cursor_hash(bytes: &[u8], offset: &mut usize) -> Result<[u8; ABI_HASH_LEN], String> {
    let end = offset
        .checked_add(ABI_HASH_LEN)
        .ok_or_else(|| "invalid hash offset".to_string())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "truncated ABI hash".to_string())?;
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(value);
    *offset = end;
    Ok(hash)
}

fn cursor_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let value = checked_u32_at(bytes, *offset)?;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| "invalid u32 offset".to_string())?;
    Ok(value)
}

fn cursor_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let value = checked_u64_at(bytes, *offset)?;
    *offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid u64 offset".to_string())?;
    Ok(value)
}

fn checked_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn checked_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn checked_u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated binary representation".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

struct BinaryReprProject {
    strings: StringPool,
    types: TypeTable,
    constants: ConstPool,
    resources: ResourceTable,
    globals: Vec<GlobalEntry>,
    manifest: BinaryReprManifest,
    imports: ImportTable,
    abi: AbiIndex,
    entry_function: u32,
    entry_flags: u32,
    functions: Vec<Function>,
    /// Structured Binary Representation payload (the faithful serialization of the source
    /// `IrProject`). This is the portable representation a consumer decodes and
    /// lowers through the single `IR -> NIR -> native` path. Function bodies are
    /// no longer flattened to opcodes; this blob is the body source of truth.
    binary_repr: Vec<u8>,
}

struct GlobalEntry {
    name: u32,
    type_id: u32,
    flags: u32,
}

struct PackageBinaryRepr {
    project: BinaryReprProject,
    exports: Vec<DecodedExport>,
}

struct BinaryReprManifest {
    package_name: u32,
    package_ident: u32,
    package_version: u32,
    ident_key: u32,
    ident_fingerprint: u32,
    signing_fingerprint: u32,
    author: u32,
    url: u32,
}

struct StringPool {
    values: Vec<String>,
}

struct TypeTable {
    entries: Vec<TypeEntry>,
    ids: HashMap<String, u32>,
}

struct TypeEntry {
    kind: u16,
    name: u32,
    owner_package: u32,
    abi_export_kind: Option<BinaryReprExportKind>,
    payload: Vec<u8>,
}

struct ConstPool {
    entries: Vec<ConstEntry>,
}

struct ConstEntry {
    kind: u16,
    payload: Vec<u8>,
}

struct ResourceTable {
    entries: Vec<ResourceEntry>,
}

struct ResourceEntry {
    type_id: u32,
    close_function_id: u32,
    flags: u32,
}

struct ImportTable {
    entries: Vec<ImportEntry>,
}

struct ImportEntry {
    package_name: u32,
    package_ident: u32,
    version: u32,
    pin: bool,
    flags: u32,
    used_symbols: Vec<AbiUsedSymbol>,
}

#[derive(Clone)]
struct AbiIndex {
    exports: Vec<AbiExport>,
    dep_edges: Vec<AbiDepEdge>,
}

#[derive(Clone)]
struct AbiExport {
    name: u32,
    kind: BinaryReprExportKind,
    sig_hash: [u8; ABI_HASH_LEN],
}

#[derive(Clone)]
struct AbiDepEdge {
    package_name: u32,
    package_ident: u32,
    version_request: u32,
    pin: bool,
    used_symbols: Vec<AbiUsedSymbol>,
}

#[derive(Clone)]
struct AbiUsedSymbol {
    name: u32,
    sig_hash: [u8; ABI_HASH_LEN],
}

struct Function {
    name: u32,
    kind: u16,
    flags: u16,
    return_type: u32,
    params: Vec<Param>,
    registers: Vec<Register>,
    cleanups: Vec<Cleanup>,
}

fn is_exported_function(function: &Function) -> bool {
    function.kind == FUNCTION_BINARY_REPR && function.flags & FUNCTION_FLAG_PRIVATE == 0
}

struct Param {
    name: u32,
    type_id: u32,
    flags: u32,
    default_const: u32,
}

struct Register {
    type_id: u32,
    flags: u32,
}

struct Cleanup {
    id: u32,
    start_pc: u32,
    end_pc: u32,
    resource_register: u32,
    close_function_id: u32,
    flags: u32,
}

fn lower_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
) -> Result<BinaryReprProject, String> {
    lower_project_with_external_functions(
        ir,
        metadata,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

fn lower_package_project(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    package_paths: &[PathBuf],
) -> Result<BinaryReprProject, String> {
    let packages = package_paths
        .iter()
        .map(|path| read_package_binary_repr(path))
        .collect::<Result<Vec<_>, _>>()?;
    let (external_function_ids, external_function_returns, external_function_abi_hashes) =
        external_function_metadata(ir.functions.len() as u32, &packages)?;
    lower_project_with_external_functions(
        ir,
        metadata,
        &external_function_ids,
        &external_function_returns,
        &external_function_abi_hashes,
    )
}

fn external_function_metadata(
    base_function_id: u32,
    packages: &[PackageBinaryRepr],
) -> Result<
    (
        HashMap<String, u32>,
        HashMap<String, String>,
        HashMap<String, [u8; ABI_HASH_LEN]>,
    ),
    String,
> {
    let mut external_function_ids = HashMap::new();
    let mut external_function_returns = HashMap::new();
    let mut external_function_abi_hashes = HashMap::new();
    let mut next_function_id = base_function_id;
    for package in packages {
        let package_name = string_at(
            &package.project.strings.values,
            package.project.manifest.package_name,
        )?;
        let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
        for export in &package.exports {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            let export_name = string_at(&package.project.strings.values, export.name)?;
            external_function_ids.insert(
                format!("{package_name}.{export_name}"),
                next_function_id + export.function_id,
            );
            external_function_returns.insert(
                format!("{package_name}.{export_name}"),
                type_name(&type_names, function.return_type)?.to_string(),
            );
            let abi_export =
                abi_export_for_decoded(&package.project.abi, export).ok_or_else(|| {
                    format!("ABI_INDEX is missing EXPORT_TABLE entry `{export_name}`")
                })?;
            external_function_abi_hashes
                .insert(format!("{package_name}.{export_name}"), abi_export.sig_hash);
        }
        next_function_id = next_function_id
            .checked_add(package.project.functions.len() as u32)
            .ok_or_else(|| "merged binary representation has too many functions".to_string())?;
    }
    Ok((
        external_function_ids,
        external_function_returns,
        external_function_abi_hashes,
    ))
}

fn lower_project_with_external_functions(
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    external_function_ids: &HashMap<String, u32>,
    external_function_returns: &HashMap<String, String>,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
) -> Result<BinaryReprProject, String> {
    let mut strings = StringPool::new();
    let ident = if metadata.ident.is_empty() {
        &metadata.name
    } else {
        &metadata.ident
    };
    let manifest = BinaryReprManifest {
        package_name: strings.intern(&metadata.name),
        package_ident: strings.intern(ident),
        package_version: strings.intern(&metadata.version),
        ident_key: strings.intern(&metadata.ident_key),
        ident_fingerprint: strings.intern(&metadata.ident_fingerprint),
        signing_fingerprint: strings.intern(&metadata.signing_fingerprint),
        author: strings.intern(&metadata.author),
        url: strings.intern(&metadata.url),
    };
    let mut imports = ImportTable::from_metadata(&mut strings, metadata);

    let mut types = TypeTable::new();
    for ir_type in &ir.types {
        types.reserve_source_type(&mut strings, &metadata.name, ir_type);
    }
    types.populate_source_payloads(&mut strings, &ir.types)?;
    let mut resources = ResourceTable::new();
    if ir_uses_resource_type(ir) {
        let mut used = HashSet::new();
        collect_resource_type_names(ir, &mut used);
        if used.contains("File") {
            resources.add_standard_file(&mut types, &mut strings);
        }
        if used.contains("Socket") {
            resources.add_standard_socket(&mut types, &mut strings);
        }
        if used.contains("Listener") {
            resources.add_standard_listener(&mut types, &mut strings);
        }
    }
    // Native LINK resources (plan-link-update.md §10): each becomes an opaque
    // type (exported when the declaration is `EXPORT`) plus a RESOURCE_TABLE
    // entry whose close op is referenced by name.
    for native in &ir.native_resources {
        // An opaque native resource has no fields; encode it as a zero-field
        // record so the type table round-trips. Its resource-ness (which blocks
        // construction and field access) comes from the RESOURCE_TABLE.
        let mut payload = Vec::new();
        put_u32(&mut payload, 0);
        let type_id = types.add_entry(&mut strings, &metadata.name, &native.name, 1, payload);
        if native.visibility == "export" {
            let index = (type_id - FIRST_TABLE_TYPE_ID) as usize;
            types.entries[index].abi_export_kind = Some(BinaryReprExportKind::Type);
        }
        resources.add_native(&mut strings, type_id, native);
    }
    let globals = ir
        .bindings
        .iter()
        .map(|binding| {
            let mut flags = 0;
            if binding.mutable {
                flags |= 1;
            }
            flags |= match binding.visibility.as_str() {
                "private" => 0 << 1,
                "package" => 1 << 1,
                "export" => 2 << 1,
                _ => 0,
            };
            GlobalEntry {
                name: strings.intern(&binding.name),
                type_id: types.type_id(&mut strings, &binding.type_),
                flags,
            }
        })
        .collect::<Vec<_>>();

    let mut constants = ConstPool::new();
    let mut function_ids = HashMap::new();
    let mut function_return_types = HashMap::new();
    let mut function_return_type_names = HashMap::new();
    for (index, function) in ir.functions.iter().enumerate() {
        function_ids.insert(function.name.clone(), index as u32);
        let return_type = types.type_id(&mut strings, &function.returns);
        function_return_types.insert(function.name.clone(), return_type);
        function_return_type_names.insert(function.name.clone(), function.returns.clone());
    }
    for (name, id) in external_function_ids {
        function_ids.insert(name.clone(), *id);
    }
    for (name, return_type_name) in external_function_returns {
        let return_type = types.type_id(&mut strings, return_type_name);
        function_return_types.insert(name.clone(), return_type);
        function_return_type_names.insert(name.clone(), return_type_name.clone());
    }

    let mut functions = Vec::new();
    let mut used_imported_functions = HashSet::new();
    for function in &ir.functions {
        functions.push(lower_function(
            function,
            &mut strings,
            &mut types,
            &mut constants,
            external_function_abi_hashes,
            &mut used_imported_functions,
        )?);
    }
    imports.record_used_imports(
        &mut strings,
        &used_imported_functions,
        external_function_abi_hashes,
    );
    let abi = AbiIndex::from_project(&strings, &types, &constants, &imports, &functions)?;

    let (entry_function, entry_flags) = if let Some(entry) = &ir.entry {
        let function_id = *function_ids.get(&entry.name).ok_or_else(|| {
            format!(
                "entry function `{}` was not lowered to binary representation",
                entry.name
            )
        })?;
        let mut flags = 1;
        if entry.accepts_args {
            flags |= 1 << 1;
        }
        if entry.returns == "Integer" {
            flags |= 1 << 2;
        }
        (function_id, flags)
    } else {
        (u32::MAX, 0)
    };

    Ok(BinaryReprProject {
        strings,
        types,
        constants,
        resources,
        globals,
        manifest,
        imports,
        abi,
        entry_function,
        entry_flags,
        functions,
        binary_repr: crate::ir::encode_binary_repr(ir),
    })
}

fn ir_uses_resource_type(ir: &IrProject) -> bool {
    ir.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_resource_type_name(&param.type_))
            || is_resource_type_name(&function.returns)
            || ops_use_resource_type(&function.body)
    })
}

fn ops_use_resource_type(ops: &[IrOp]) -> bool {
    ops.iter().any(|op| match op {
        IrOp::Bind { type_, value, .. } => {
            is_resource_type_name(type_) || value.as_ref().is_some_and(value_uses_resource_type)
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value } => value_uses_resource_type(value),
        IrOp::Return { value } => value.as_ref().is_some_and(value_uses_resource_type),
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
        IrOp::ExitProgram { code } => value_uses_resource_type(code),
        IrOp::Fail { error } => value_uses_resource_type(error),
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_resource_type(condition)
                || ops_use_resource_type(then_body)
                || ops_use_resource_type(else_body)
        }
        IrOp::Match { value, cases } => {
            value_uses_resource_type(value)
                || cases.iter().any(|case| ops_use_resource_type(&case.body))
        }
        IrOp::While {
            condition, body, ..
        } => value_uses_resource_type(condition) || ops_use_resource_type(body),
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_resource_type(start)
                || value_uses_resource_type(end)
                || value_uses_resource_type(step)
                || ops_use_resource_type(body)
        }
        IrOp::DoUntil { body, condition } => {
            ops_use_resource_type(body) || value_uses_resource_type(condition)
        }
        IrOp::ForEach {
            type_,
            iterable,
            body,
            ..
        } => {
            is_resource_type_name(type_)
                || value_uses_resource_type(iterable)
                || ops_use_resource_type(body)
        }
        IrOp::Trap { body, .. } => ops_use_resource_type(body),
    })
}

fn value_uses_resource_type(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => is_resource_type_name(type_),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            builtins::call_return_type_name(target).is_some_and(is_resource_type_name)
                || args.iter().any(value_uses_resource_type)
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value } => value_uses_resource_type(value),
        IrValue::MemberAccess { target, .. } => value_uses_resource_type(target),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_resource_type(target)
                || updates
                    .iter()
                    .any(|update| value_uses_resource_type(&update.value))
        }
        IrValue::Binary { left, right, .. } => {
            value_uses_resource_type(left) || value_uses_resource_type(right)
        }
        IrValue::Unary { operand, .. } => value_uses_resource_type(operand),
        IrValue::Local(_) | IrValue::Global(_) => false,
    }
}

fn is_resource_type_name(type_name: &str) -> bool {
    builtins::is_resource_type(type_name)
}

/// Collect the bare resource type names (`File`, `Socket`, `Listener`) actually
/// referenced by the project so only the resource tables that are used get
/// emitted. Resource handles cannot appear inside collections, so resource type
/// strings are always bare names.
fn collect_resource_type_names(ir: &IrProject, names: &mut HashSet<String>) {
    let mut record = |type_: &str, names: &mut HashSet<String>| {
        if is_resource_type_name(type_) {
            names.insert(type_.to_string());
        }
    };
    for function in &ir.functions {
        for param in &function.params {
            record(&param.type_, names);
        }
        record(&function.returns, names);
        collect_resource_names_in_ops(&function.body, names, &mut record);
    }
}

fn collect_resource_names_in_ops(
    ops: &[IrOp],
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    for op in ops {
        match op {
            IrOp::Bind { type_, value, .. } => {
                record(type_, names);
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value } => collect_resource_names_in_value(value, names, record),
            IrOp::Return { value } => {
                if let Some(value) = value {
                    collect_resource_names_in_value(value, names, record);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::ExitProgram { code } => collect_resource_names_in_value(code, names, record),
            IrOp::Fail { error } => collect_resource_names_in_value(error, names, record),
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(then_body, names, record);
                collect_resource_names_in_ops(else_body, names, record);
            }
            IrOp::Match { value, cases } => {
                collect_resource_names_in_value(value, names, record);
                for case in cases {
                    collect_resource_names_in_ops(&case.body, names, record);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                collect_resource_names_in_value(condition, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_resource_names_in_value(start, names, record);
                collect_resource_names_in_value(end, names, record);
                collect_resource_names_in_value(step, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::DoUntil { body, condition } => {
                collect_resource_names_in_ops(body, names, record);
                collect_resource_names_in_value(condition, names, record);
            }
            IrOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                record(type_, names);
                collect_resource_names_in_value(iterable, names, record);
                collect_resource_names_in_ops(body, names, record);
            }
            IrOp::Trap { body, .. } => collect_resource_names_in_ops(body, names, record),
        }
    }
}

fn collect_resource_names_in_value(
    value: &IrValue,
    names: &mut HashSet<String>,
    record: &mut impl FnMut(&str, &mut HashSet<String>),
) {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Closure { type_, .. }
        | IrValue::Capture { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => record(type_, names),
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if let Some(returns) = builtins::call_return_type_name(target) {
                record(returns, names);
            }
            for arg in args {
                collect_resource_names_in_value(arg, names, record);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value } => {
            collect_resource_names_in_value(value, names, record)
        }
        IrValue::MemberAccess { target, .. } => {
            collect_resource_names_in_value(target, names, record)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_resource_names_in_value(target, names, record);
            for update in updates {
                collect_resource_names_in_value(&update.value, names, record);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_resource_names_in_value(left, names, record);
            collect_resource_names_in_value(right, names, record);
        }
        IrValue::Unary { operand, .. } => {
            collect_resource_names_in_value(operand, names, record)
        }
        IrValue::Local(_) | IrValue::Global(_) => {}
    }
}

/// Lower an `IrFunction` to its container *metadata* (`Function`): name, kind,
/// flags, return type, and parameter signatures. Function *bodies* are no longer
/// flattened to opcodes here — they are carried verbatim in the structured
/// Binary Representation payload (`SECTION_BINARY_REPR`). The flat `code`/`registers`/`cleanups`
/// fields are therefore empty; only the signature-level tables (function table,
/// export table, ABI index, import table) consume this metadata.
fn lower_function(
    function: &IrFunction,
    strings: &mut StringPool,
    types: &mut TypeTable,
    constants: &mut ConstPool,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used_imported_functions: &mut HashSet<String>,
) -> Result<Function, String> {
    let mut params = Vec::new();
    for param in &function.params {
        let type_id = types.type_id(strings, &param.type_);
        params.push(Param {
            name: strings.intern(&param.name),
            type_id,
            flags: if param.default.is_some() { 1 } else { 0 },
            default_const: match &param.default {
                Some(default) => constants.add(strings, default)?,
                None => u32::MAX,
            },
        });
    }

    // Record which imported (cross-package) functions this body references so the
    // import table can pin the exact used symbols. Imported targets are exactly
    // the qualified names present in `external_function_abi_hashes`.
    for op in &function.body {
        collect_imported_calls_op(op, external_function_abi_hashes, used_imported_functions);
    }

    let mut flags = if function.visibility == "export" {
        0
    } else {
        FUNCTION_FLAG_PRIVATE
    };
    if function.kind == "sub" {
        flags |= FUNCTION_FLAG_SUB | FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.returns == "Nothing" {
        flags |= FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.isolated {
        flags |= FUNCTION_FLAG_ISOLATED;
    }

    Ok(Function {
        name: strings.intern(&function.name),
        kind: FUNCTION_BINARY_REPR,
        flags,
        return_type: types.type_id(strings, &function.returns),
        params,
        registers: Vec::new(),
        cleanups: Vec::new(),
    })
}

/// Walk an `IrOp`, recording any call/reference target that names an imported
/// (cross-package) function into `used`.
fn collect_imported_calls_op(
    op: &IrOp,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value }
        | IrOp::Fail { error: value } => collect_imported_calls_value(value, imported, used),
        IrOp::Return { value } => {
            if let Some(v) = value {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code } => collect_imported_calls_value(code, imported, used),
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in then_body.iter().chain(else_body) {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Match { value, cases } => {
            collect_imported_calls_value(value, imported, used);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_imported_calls_value(guard, imported, used);
                }
                for op in &case.body {
                    collect_imported_calls_op(op, imported, used);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            collect_imported_calls_value(condition, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_imported_calls_value(start, imported, used);
            collect_imported_calls_value(end, imported, used);
            collect_imported_calls_value(step, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::DoUntil { body, condition } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
            collect_imported_calls_value(condition, imported, used);
        }
        IrOp::ForEach { iterable, body, .. } => {
            collect_imported_calls_value(iterable, imported, used);
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                collect_imported_calls_op(op, imported, used);
            }
        }
    }
}

/// Walk an `IrValue`, recording imported function references into `used`.
fn collect_imported_calls_value(
    value: &IrValue,
    imported: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used: &mut HashSet<String>,
) {
    let note = |target: &str, used: &mut HashSet<String>| {
        if imported.contains_key(target) {
            used.insert(target.to_string());
        }
    };
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            note(target, used);
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::FunctionRef { name, .. } => note(name, used),
        IrValue::Closure { name, captures, .. } => {
            note(name, used);
            for capture in captures {
                collect_imported_calls_value(capture, imported, used);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                collect_imported_calls_value(arg, imported, used);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            collect_imported_calls_value(value, imported, used)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_imported_calls_value(target, imported, used);
            for update in updates {
                collect_imported_calls_value(&update.value, imported, used);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_imported_calls_value(k, imported, used);
                collect_imported_calls_value(v, imported, used);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_imported_calls_value(left, imported, used);
            collect_imported_calls_value(right, imported, used);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::Capture { .. } => {}
    }
}

#[derive(Clone)]
struct FunctionTypeSignature {
    isolated: bool,
    params: Vec<String>,
    returns: String,
}

fn parse_function_type(type_name: &str) -> Option<FunctionTypeSignature> {
    let (isolated, rest) = if let Some(rest) = type_name.strip_prefix("ISOLATED FUNC(") {
        (true, rest)
    } else {
        (false, type_name.strip_prefix("FUNC(")?)
    };
    let (params, returns) = split_function_type_rest(rest)?;
    Some(FunctionTypeSignature {
        isolated,
        params: split_top_level_types(params),
        returns: returns.to_string(),
    })
}

fn split_function_type_rest(rest: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let bytes = rest.as_bytes();
    for index in 0..bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth == 0 && rest[index..].starts_with(") AS ") => {
                return Some((&rest[..index], &rest[index + 5..]));
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(params[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(params[start..].trim().to_string());
    result
}

impl StringPool {
    fn new() -> Self {
        Self { values: Vec::new() }
    }

    fn intern(&mut self, value: &str) -> u32 {
        if let Some(index) = self.values.iter().position(|existing| existing == value) {
            return index as u32;
        }
        let index = self.values.len() as u32;
        self.values.push(value.to_string());
        index
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.values.len() as u32);
        for value in &self.values {
            put_bytes(&mut bytes, value.as_bytes());
        }
        bytes
    }
}

impl TypeTable {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            ids: HashMap::new(),
        }
    }

    fn reserve_source_type(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        ir_type: &IrType,
    ) -> u32 {
        let (kind, abi_export_kind) = match ir_type.kind.as_str() {
            "type" => (1, BinaryReprExportKind::Type),
            "union" => (2, BinaryReprExportKind::Union),
            "enum" => (3, BinaryReprExportKind::Enum),
            _ => (1, BinaryReprExportKind::Type),
        };
        let id = self.add_entry(strings, package, &ir_type.name, kind, Vec::new());
        if ir_type.visibility == "export" {
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].abi_export_kind =
                Some(abi_export_kind);
        }
        id
    }

    fn populate_source_payloads(
        &mut self,
        strings: &mut StringPool,
        ir_types: &[IrType],
    ) -> Result<(), String> {
        let source_types = ir_types
            .iter()
            .map(|ir_type| (ir_type.name.as_str(), ir_type))
            .collect::<HashMap<_, _>>();

        for ir_type in ir_types {
            let id = *self
                .ids
                .get(&ir_type.name)
                .ok_or_else(|| format!("source type `{}` was not reserved", ir_type.name))?;
            let payload = source_type_payload(strings, self, &source_types, ir_type)?;
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].payload = payload;
        }

        Ok(())
    }

    fn type_id(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        match name {
            "Nothing" => TYPE_NOTHING,
            "Boolean" => TYPE_BOOLEAN,
            "Integer" => TYPE_INTEGER,
            "Float" => TYPE_FLOAT,
            "Fixed" => TYPE_FIXED,
            "String" => TYPE_STRING,
            "File" => TYPE_FILE_HANDLE,
            "Socket" => TYPE_SOCKET_HANDLE,
            "Listener" => TYPE_LISTENER_HANDLE,
            name if name.starts_with("List OF ") => {
                let element = self.type_id(strings, name.trim_start_matches("List OF "));
                self.list_type(strings, element)
            }
            name if name.starts_with("Result OF ") => {
                let success = self.type_id(strings, name.trim_start_matches("Result OF "));
                self.result_type(strings, success)
            }
            name if name.starts_with("Thread OF ") => {
                if let Some((_, message, resource, output)) =
                    builtins::thread::thread_parts_full(name)
                {
                    let message = self.type_id(strings, message);
                    let resource = resource.map(|resource| self.type_id(strings, resource));
                    let output = self.type_id(strings, output);
                    self.thread_type(strings, message, resource, output)
                } else {
                    self.add_entry(strings, "", name, 7, Vec::new())
                }
            }
            name if name.starts_with("ThreadWorker OF ") => {
                if let Some((_, message, resource, output)) =
                    builtins::thread::thread_parts_full(name)
                {
                    let message = self.type_id(strings, message);
                    let resource = resource.map(|resource| self.type_id(strings, resource));
                    let output = self.type_id(strings, output);
                    self.thread_worker_type(strings, message, resource, output)
                } else {
                    self.add_entry(strings, "", name, 10, Vec::new())
                }
            }
            name if name.starts_with("FUNC(") => self.function_type(strings, name),
            name if name.starts_with("ISOLATED FUNC(") => self.function_type(strings, name),
            name if name.starts_with("Map OF ") => {
                let rest = name.trim_start_matches("Map OF ");
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 5, Vec::new())
                }
            }
            name if name.starts_with("MapEntry OF ") => {
                let rest = name.trim_start_matches("MapEntry OF ");
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_entry_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 9, Vec::new())
                }
            }
            "Byte" => TYPE_BYTE,
            "Error" => {
                strings.intern("code");
                strings.intern("message");
                TYPE_ERROR
            }
            "TerminalSize" => {
                strings.intern("columns");
                strings.intern("rows");
                TYPE_TERMINAL_SIZE
            }
            _ => {
                if let Some(id) = self.ids.get(name) {
                    *id
                } else {
                    self.add_entry(strings, "", name, 1, Vec::new())
                }
            }
        }
    }

    fn result_type(&mut self, strings: &mut StringPool, success_type: u32) -> u32 {
        let name = format!("Result#{success_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, success_type);
        self.add_entry(strings, "", &name, 6, payload)
    }

    fn list_type(&mut self, strings: &mut StringPool, element_type: u32) -> u32 {
        let name = format!("List#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 4, payload)
    }

    fn map_type(&mut self, strings: &mut StringPool, key_type: u32, value_type: u32) -> u32 {
        let name = format!("Map#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 5, payload)
    }

    fn map_entry_type(&mut self, strings: &mut StringPool, key_type: u32, value_type: u32) -> u32 {
        let name = format!("MapEntry#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 9, payload)
    }

    fn function_type(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let mut payload = Vec::new();
        if let Some(signature) = parse_function_type(name) {
            put_u32(&mut payload, if signature.isolated { 1 } else { 0 });
            put_u32(&mut payload, signature.params.len() as u32);
            let return_type = self.type_id(strings, &signature.returns);
            put_u32(&mut payload, return_type);
            for param in signature.params {
                let param_type = self.type_id(strings, &param);
                put_u32(&mut payload, param_type);
            }
        }
        self.add_entry(strings, "", name, 8, payload)
    }

    fn thread_type(
        &mut self,
        strings: &mut StringPool,
        message_type: u32,
        resource_type: Option<u32>,
        output_type: u32,
    ) -> u32 {
        // A data-only thread encodes exactly as before (message, output); the
        // resource type-id is appended only when the resource plane is present,
        // keeping data-only packages byte-compatible.
        let resource_key = resource_type.map_or(String::new(), |id| format!("#r{id}"));
        let name = format!("Thread#{message_type}#{output_type}{resource_key}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, message_type);
        put_u32(&mut payload, output_type);
        if let Some(resource_type) = resource_type {
            put_u32(&mut payload, resource_type);
        }
        self.add_entry(strings, "thread", &name, 7, payload)
    }

    fn thread_worker_type(
        &mut self,
        strings: &mut StringPool,
        message_type: u32,
        resource_type: Option<u32>,
        output_type: u32,
    ) -> u32 {
        let resource_key = resource_type.map_or(String::new(), |id| format!("#r{id}"));
        let name = format!("ThreadWorker#{message_type}#{output_type}{resource_key}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, message_type);
        put_u32(&mut payload, output_type);
        if let Some(resource_type) = resource_type {
            put_u32(&mut payload, resource_type);
        }
        self.add_entry(strings, "thread", &name, 10, payload)
    }

    fn add_entry(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        name: &str,
        kind: u16,
        payload: Vec<u8>,
    ) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = FIRST_TABLE_TYPE_ID + self.entries.len() as u32;
        self.ids.insert(name.to_string(), id);
        self.entries.push(TypeEntry {
            kind,
            name: strings.intern(name),
            owner_package: strings.intern(package),
            abi_export_kind: None,
            payload,
        });
        id
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let entry_bytes = 20usize;
        let mut payload_offset = 4 + self.entries.len() * entry_bytes;
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, entry.name);
            put_u32(&mut bytes, entry.owner_package);
            put_u32(&mut bytes, payload_offset as u32);
            put_u32(&mut bytes, entry.payload.len() as u32);
            payload_offset += entry.payload.len();
        }
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.payload);
        }
        bytes
    }
}

fn source_type_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    source_types: &HashMap<&str, &IrType>,
    ir_type: &IrType,
) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    match ir_type.kind.as_str() {
        "type" => {
            put_u32(&mut payload, ir_type.fields.len() as u32);
            for field in &ir_type.fields {
                put_field_payload(strings, types, &mut payload, field);
            }
        }
        "union" => {
            let variants = concrete_union_variants(source_types, ir_type)?;
            put_u32(&mut payload, variants.len() as u32);
            for variant in variants {
                put_u32(&mut payload, strings.intern(&variant.name));
                put_u32(&mut payload, variant.fields.len() as u32);
                for field in &variant.fields {
                    put_u32(&mut payload, strings.intern(&field.name));
                    put_u32(&mut payload, types.type_id(strings, &field.type_));
                }
            }
        }
        "enum" => {
            put_u32(&mut payload, ir_type.members.len() as u32);
            for (ordinal, member) in ir_type.members.iter().enumerate() {
                put_u32(&mut payload, strings.intern(&member.name));
                put_u32(&mut payload, ordinal as u32);
            }
        }
        _ => {}
    }
    Ok(payload)
}

fn concrete_union_variants<'a>(
    source_types: &HashMap<&str, &'a IrType>,
    ir_type: &'a IrType,
) -> Result<Vec<&'a crate::ir::IrVariant>, String> {
    let mut variants = Vec::new();
    for include in &ir_type.includes {
        let included = source_types.get(include.as_str()).ok_or_else(|| {
            format!(
                "union `{}` includes unknown union `{include}`",
                ir_type.name
            )
        })?;
        variants.extend(concrete_union_variants(source_types, included)?);
    }
    variants.extend(ir_type.variants.iter());
    Ok(variants)
}

fn put_field_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    payload: &mut Vec<u8>,
    field: &crate::ir::IrField,
) {
    put_u32(payload, strings.intern(&field.name));
    put_u32(payload, types.type_id(strings, &field.type_));
    put_u32(
        payload,
        match field.visibility.as_deref() {
            Some("private") => 1,
            Some("package") => 2,
            Some("export") => 3,
            _ => 0,
        },
    );
}

impl ConstPool {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn add(&mut self, strings: &mut StringPool, value: &IrValue) -> Result<u32, String> {
        let entry = match value {
            IrValue::Const { type_, value } => match type_.as_str() {
                "Nothing" => ConstEntry {
                    kind: 1,
                    payload: Vec::new(),
                },
                "String" => {
                    let mut payload = Vec::new();
                    put_u32(&mut payload, strings.intern(value));
                    ConstEntry { kind: 6, payload }
                }
                "Integer" => ConstEntry {
                    kind: 3,
                    payload: value
                        .parse::<i64>()
                        .map_err(|_| format!("invalid Integer constant `{value}`"))?
                        .to_le_bytes()
                        .to_vec(),
                },
                "Float" => ConstEntry {
                    kind: 4,
                    payload: value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid Float constant `{value}`"))?
                        .to_bits()
                        .to_le_bytes()
                        .to_vec(),
                },
                "Fixed" => ConstEntry {
                    kind: 5,
                    payload: fixed_raw_from_decimal(value)?.to_le_bytes().to_vec(),
                },
                "Boolean" => ConstEntry {
                    kind: 2,
                    payload: vec![if value == "true" { 1 } else { 0 }],
                },
                "Byte" => ConstEntry {
                    kind: 7,
                    payload: vec![value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid Byte constant `{value}`"))?],
                },
                _ => return Err(format!("unsupported constant type `{type_}`")),
            },
            _ => return Err("only constant IR values can be stored in CONST_POOL".to_string()),
        };

        let id = self.entries.len() as u32;
        self.entries.push(entry);
        Ok(id)
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_bytes(&mut bytes, &entry.payload);
        }
        bytes
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

/// The `RESOURCE_TABLE` flags for a standard built-in resource, including the
/// "sendable to thread" bit (bit 2) when the registry marks the type sendable.
fn standard_resource_flags(type_name: &str) -> u32 {
    let mut flags = RESOURCE_FLAG_NATIVE | RESOURCE_FLAG_STANDARD | RESOURCE_FLAG_CLOSE_MAY_FAIL;
    if builtins::resource::is_builtin_sendable_resource_type(type_name) {
        flags |= RESOURCE_FLAG_SENDABLE;
    }
    flags
}

impl ResourceTable {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn add_standard_file(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_id = types.type_id(strings, builtins::fs::FILE_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::fs::FILE_TYPE),
        });
    }

    fn add_standard_socket(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_id = types.type_id(strings, builtins::net::SOCKET_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::net::SOCKET_TYPE),
        });
    }

    fn add_standard_listener(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_id = types.type_id(strings, builtins::net::LISTENER_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
            flags: standard_resource_flags(builtins::net::LISTENER_TYPE),
        });
    }

    /// Add a native LINK resource (plan-link-update.md §10). Native resources
    /// carry the `NATIVE` flag *without* `STANDARD`, which is how decode tells a
    /// native LINK resource (whose `close_function_id` is the string id of its
    /// close op name) from a built-in (whose id is a sentinel).
    fn add_native(
        &mut self,
        strings: &mut StringPool,
        type_id: u32,
        native: &crate::ir::IrNativeResource,
    ) {
        let mut flags = RESOURCE_FLAG_NATIVE;
        if native.sendable {
            flags |= RESOURCE_FLAG_SENDABLE;
        }
        if native.close_may_fail {
            flags |= RESOURCE_FLAG_CLOSE_MAY_FAIL;
        }
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: strings.intern(&native.close_function),
            flags,
        });
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.type_id);
            put_u32(&mut bytes, entry.close_function_id);
            put_u32(&mut bytes, entry.flags);
        }
        bytes
    }
}

impl ImportTable {
    fn from_metadata(strings: &mut StringPool, metadata: &BinaryReprMetadata) -> Self {
        let entries = metadata
            .dependencies
            .iter()
            .map(|dependency| ImportEntry {
                package_name: strings.intern(&dependency.name),
                package_ident: strings.intern(if dependency.ident.is_empty() {
                    &dependency.name
                } else {
                    &dependency.ident
                }),
                version: strings.intern(&dependency.version),
                pin: dependency.pin,
                flags: dependency.flags,
                used_symbols: Vec::new(),
            })
            .collect();

        Self { entries }
    }

    fn record_used_imports(
        &mut self,
        strings: &mut StringPool,
        used_imported_functions: &HashSet<String>,
        external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    ) {
        let import_names = self
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.package_name,
                    strings.values[entry.package_name as usize].clone(),
                )
            })
            .collect::<Vec<_>>();

        for (package_name_id, package_name) in import_names {
            let prefix = format!("{package_name}.");
            let mut symbols = used_imported_functions
                .iter()
                .filter_map(|target| {
                    let symbol_name = target.strip_prefix(&prefix)?;
                    let sig_hash = *external_function_abi_hashes.get(target)?;
                    Some(AbiUsedSymbol {
                        name: strings.intern(symbol_name),
                        sig_hash,
                    })
                })
                .collect::<Vec<_>>();
            symbols.sort_by_key(|symbol| strings.values[symbol.name as usize].clone());
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.package_name == package_name_id)
            {
                entry.used_symbols = symbols;
            }
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.package_name);
            put_u32(&mut bytes, entry.package_ident);
            put_u32(&mut bytes, entry.version);
            bytes.push(if entry.pin { 1 } else { 0 });
            put_u32(&mut bytes, entry.flags);
            put_u32(&mut bytes, entry.used_symbols.len() as u32);
            for symbol in &entry.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

impl AbiIndex {
    fn from_project(
        strings: &StringPool,
        types: &TypeTable,
        constants: &ConstPool,
        imports: &ImportTable,
        functions: &[Function],
    ) -> Result<Self, String> {
        let mut exports = Vec::new();
        for function in functions {
            if !is_exported_function(function) {
                continue;
            }
            let kind = if function.flags & FUNCTION_FLAG_SUB != 0 {
                BinaryReprExportKind::Sub
            } else {
                BinaryReprExportKind::Func
            };
            exports.push(AbiExport {
                name: function.name,
                kind,
                sig_hash: function_sig_hash(function, kind, &strings.values, types, constants)?,
            });
        }
        for (index, type_) in types.entries.iter().enumerate() {
            let Some(kind) = type_.abi_export_kind else {
                continue;
            };
            exports.push(AbiExport {
                name: type_.name,
                kind,
                sig_hash: type_sig_hash(
                    FIRST_TABLE_TYPE_ID + index as u32,
                    kind,
                    &strings.values,
                    types,
                    constants,
                )?,
            });
        }

        let dep_edges = imports
            .entries
            .iter()
            .map(|entry| AbiDepEdge {
                package_name: entry.package_name,
                package_ident: entry.package_ident,
                version_request: entry.version,
                pin: entry.pin,
                used_symbols: entry.used_symbols.clone(),
            })
            .collect();

        Ok(Self { exports, dep_edges })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u16(&mut bytes, ABI_FORMAT_VERSION);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.exports.len() as u32);
        for export in &self.exports {
            put_u32(&mut bytes, export.name);
            put_u16(&mut bytes, encode_export_kind(export.kind));
            bytes.extend_from_slice(&export.sig_hash);
        }
        put_u32(&mut bytes, self.dep_edges.len() as u32);
        for edge in &self.dep_edges {
            put_u32(&mut bytes, edge.package_name);
            put_u32(&mut bytes, edge.package_ident);
            put_u32(&mut bytes, edge.version_request);
            bytes.push(if edge.pin { 1 } else { 0 });
            put_u32(&mut bytes, edge.used_symbols.len() as u32);
            for symbol in &edge.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

impl BinaryReprProject {
    fn encode(&self) -> Vec<u8> {
        // Function bodies live in the structured Binary Representation payload, not a flat
        // code stream. The function table still records signatures/metadata, so
        // every per-function code region is zero-length.
        let code_offsets: Vec<(u64, u64)> = self.functions.iter().map(|_| (0, 0)).collect();

        let mut sections = vec![
            Section::new(SECTION_MANIFEST, self.encode_manifest()),
            Section::new(SECTION_STRING_POOL, self.strings.encode()),
            Section::new(SECTION_TYPE_TABLE, self.types.encode()),
            Section::new(SECTION_CONST_POOL, self.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, self.imports.encode()),
            Section::new(SECTION_EXPORT_TABLE, self.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, self.encode_globals()),
            Section::new(SECTION_FUNCTION_TABLE, self.encode_functions(&code_offsets)),
            Section::new(SECTION_BINARY_REPR, self.binary_repr.clone()),
            Section::new(SECTION_ABI_INDEX, self.abi.encode()),
        ];
        if !self.resources.entries.is_empty() {
            sections.push(Section::new(
                SECTION_RESOURCE_TABLE,
                self.resources.encode(),
            ));
        }

        encode_sections(&sections)
    }

    fn encode_manifest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.manifest.package_name);
        put_u32(&mut bytes, self.manifest.package_ident);
        put_u32(&mut bytes, self.manifest.package_version);
        put_u32(&mut bytes, self.manifest.ident_key);
        put_u32(&mut bytes, self.manifest.ident_fingerprint);
        put_u32(&mut bytes, self.manifest.signing_fingerprint);
        put_u32(&mut bytes, self.manifest.author);
        put_u32(&mut bytes, self.manifest.url);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.imports.entries.len() as u32);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, self.export_count());
        put_u32(&mut bytes, self.entry_function);
        put_u32(&mut bytes, self.entry_flags);
        bytes
    }

    fn encode_exports(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.export_count());
        for (index, function) in self.functions.iter().enumerate() {
            if !is_exported_function(function) {
                continue;
            }
            put_u32(&mut bytes, function.name);
            put_u16(
                &mut bytes,
                if function.flags & FUNCTION_FLAG_SUB != 0 {
                    2
                } else {
                    1
                },
            );
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, index as u32);
        }
        bytes
    }

    fn encode_globals(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.globals.len() as u32);
        for global in &self.globals {
            put_u32(&mut bytes, global.name);
            put_u32(&mut bytes, global.type_id);
            put_u32(&mut bytes, global.flags);
        }
        bytes
    }

    fn export_count(&self) -> u32 {
        self.functions
            .iter()
            .filter(|function| is_exported_function(function))
            .count() as u32
    }

    fn encode_functions(&self, code_offsets: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.functions.len() as u32);
        for (index, function) in self.functions.iter().enumerate() {
            let (code_offset, code_length) = code_offsets[index];
            put_u32(&mut bytes, function.name);
            put_u16(&mut bytes, function.kind);
            put_u16(&mut bytes, function.flags);
            put_u32(&mut bytes, function.params.len() as u32);
            put_u32(&mut bytes, function.return_type);
            put_u32(&mut bytes, function.registers.len() as u32);
            put_u64(&mut bytes, code_offset);
            put_u64(&mut bytes, code_length);
            put_u32(&mut bytes, u32::MAX);
            put_u32(&mut bytes, function.cleanups.len() as u32);
            let cleanup_offset =
                bytes.len() + 8 + function.params.len() * 16 + function.registers.len() * 8;
            put_u64(
                &mut bytes,
                if function.cleanups.is_empty() {
                    0
                } else {
                    cleanup_offset as u64
                },
            );

            for param in &function.params {
                put_u32(&mut bytes, param.name);
                put_u32(&mut bytes, param.type_id);
                put_u32(&mut bytes, param.flags);
                put_u32(&mut bytes, param.default_const);
            }

            for register in &function.registers {
                put_u32(&mut bytes, register.type_id);
                put_u32(&mut bytes, register.flags);
            }

            for cleanup in &function.cleanups {
                put_u32(&mut bytes, cleanup.id);
                put_u32(&mut bytes, cleanup.start_pc);
                put_u32(&mut bytes, cleanup.end_pc);
                put_u32(&mut bytes, cleanup.resource_register);
                put_u32(&mut bytes, cleanup.close_function_id);
                put_u32(&mut bytes, cleanup.flags);
            }
        }
        bytes
    }
}

struct Section {
    id: u16,
    data: Vec<u8>,
}

impl Section {
    fn new(id: u16, data: Vec<u8>) -> Self {
        Self { id, data }
    }
}

fn encode_sections(sections: &[Section]) -> Vec<u8> {
    let section_table_size = sections.len() * 24;
    let mut offset = 16 + section_table_size;
    let mut bytes = Vec::new();

    bytes.extend_from_slice(b"MFPC");
    put_u16(&mut bytes, MFPC_MAJOR_VERSION);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, sections.len() as u32);

    for section in sections {
        put_u16(&mut bytes, section.id);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u64(&mut bytes, offset as u64);
        put_u64(&mut bytes, section.data.len() as u64);
        offset += section.data.len();
    }

    for section in sections {
        bytes.extend_from_slice(&section.data);
    }

    bytes
}

fn hex_dump(bytes: &[u8]) -> String {
    let mut output = String::new();
    for chunk in bytes.chunks(16) {
        for (index, byte) in chunk.iter().enumerate() {
            if index > 0 {
                output.push(' ');
            }
            output.push_str(&format!("{byte:02X}"));
        }
        output.push('\n');
    }
    output
}

fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(dst, bytes.len() as u32);
    dst.extend_from_slice(bytes);
}

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod resource_table_tests {
    use super::*;

    #[test]
    fn standard_flags_set_sendable_bit_for_movable_resources() {
        let file = standard_resource_flags(builtins::fs::FILE_TYPE);
        let socket = standard_resource_flags(builtins::net::SOCKET_TYPE);
        let listener = standard_resource_flags(builtins::net::LISTENER_TYPE);
        assert!(file & RESOURCE_FLAG_SENDABLE != 0, "File must be sendable");
        assert!(socket & RESOURCE_FLAG_SENDABLE != 0, "Socket must be sendable");
        assert!(
            listener & RESOURCE_FLAG_SENDABLE == 0,
            "Listener must not be sendable"
        );
        // The other standard flags remain set.
        for flags in [file, socket, listener] {
            assert!(flags & RESOURCE_FLAG_NATIVE != 0);
            assert!(flags & RESOURCE_FLAG_STANDARD != 0);
            assert!(flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0);
        }
    }

    #[test]
    fn resource_table_round_trips_flags() {
        let table = ResourceTable {
            entries: vec![
                ResourceEntry {
                    type_id: 10,
                    close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
                    flags: standard_resource_flags(builtins::fs::FILE_TYPE),
                },
                ResourceEntry {
                    type_id: 11,
                    close_function_id: BUILTIN_NET_CLOSE_FUNCTION_ID,
                    flags: standard_resource_flags(builtins::net::LISTENER_TYPE),
                },
            ],
        };
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.entries[0].type_id, 10);
        assert_eq!(
            decoded.entries[0].close_function_id,
            BUILTIN_FS_CLOSE_FUNCTION_ID
        );
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_SENDABLE != 0);
        assert!(decoded.entries[1].flags & RESOURCE_FLAG_SENDABLE == 0);
        assert_eq!(decoded.entries[1].close_function_id, BUILTIN_NET_CLOSE_FUNCTION_ID);
    }

    #[test]
    fn native_resource_entry_has_native_flag_without_standard() {
        // A native LINK resource carries NATIVE but not STANDARD; this is how
        // decode tells it from a built-in (plan-link-update.md §10).
        let mut strings = StringPool::new();
        let mut table = ResourceTable::new();
        let native = crate::ir::IrNativeResource {
            name: "Db".to_string(),
            visibility: "export".to_string(),
            close_function: "sqliteLink.close".to_string(),
            sendable: false,
            close_may_fail: true,
        };
        table.add_native(&mut strings, 42, &native);
        let entry = &table.entries[0];
        assert!(entry.flags & RESOURCE_FLAG_NATIVE != 0);
        assert!(entry.flags & RESOURCE_FLAG_STANDARD == 0);
        assert!(entry.flags & RESOURCE_FLAG_CLOSE_MAY_FAIL != 0);
        assert!(entry.flags & RESOURCE_FLAG_SENDABLE == 0);
        // The close op name round-trips through the string pool.
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert_eq!(decoded.entries[0].type_id, 42);
        assert_eq!(
            string_at(&strings.values, decoded.entries[0].close_function_id).unwrap(),
            "sqliteLink.close"
        );
    }

    #[test]
    fn native_resource_sendable_bit_round_trips() {
        let mut strings = StringPool::new();
        let mut table = ResourceTable::new();
        let native = crate::ir::IrNativeResource {
            name: "Conn".to_string(),
            visibility: "export".to_string(),
            close_function: "lib.close".to_string(),
            sendable: true,
            close_may_fail: false,
        };
        table.add_native(&mut strings, 7, &native);
        let bytes = table.encode();
        let decoded = read_resource_table(&bytes).expect("decode resource table");
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_SENDABLE != 0);
        assert!(decoded.entries[0].flags & RESOURCE_FLAG_CLOSE_MAY_FAIL == 0);
    }
}
