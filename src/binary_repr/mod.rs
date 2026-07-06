use crate::builtins;
use crate::ir::{IrFunction, IrOp, IrProject, IrType, IrValue};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

mod builder;
mod reader;
mod sections;
#[cfg(test)]
mod tests;
mod util;
mod writer;

use builder::*;
use reader::*;
use util::*;
use writer::*;

const SECTION_MANIFEST: u16 = 1;
const SECTION_STRING_POOL: u16 = 2;
const SECTION_TYPE_TABLE: u16 = 3;
const SECTION_CONST_POOL: u16 = 4;
const SECTION_IMPORT_TABLE: u16 = 5;
const SECTION_EXPORT_TABLE: u16 = 6;
const SECTION_GLOBAL_TABLE: u16 = 7;
const SECTION_FUNCTION_TABLE: u16 = 8;
const SECTION_RESOURCE_TABLE: u16 = 11;
/// Optional documentation section (plan-09-doc.md §5). Self-describing and
/// length-prefixed; a consumer that does not understand it skips it entirely.
/// Ids 12-14 are reserved by the format for DEBUG_INFO/SOURCE_MAP/AUDIT_INFO,
/// so the doc table takes the next free id past the IR section.
const SECTION_DOC_TABLE: u16 = 17;
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
// `term::` builtin record types live in the high reserved id range alongside the
// handle types (File/Socket/Listener), not the low primitive range: the only
// freed low slot is id 9 (the removed `TerminalSize`), and ids at/above
// `FIRST_TABLE_TYPE_ID` (10) would collide with per-package user/table type ids,
// silently hijacking another package's first table type in the signature hash.
pub(crate) const TYPE_FILE_HANDLE: u32 = 0xffff_ff00;
pub(crate) const TYPE_SOCKET_HANDLE: u32 = 0xffff_feff;
pub(crate) const TYPE_LISTENER_HANDLE: u32 = 0xffff_fefe;
pub(crate) const TYPE_TERM_COLOR: u32 = 0xffff_fefd;
pub(crate) const TYPE_TERM_SIZE: u32 = 0xffff_fefc;
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
    Public,
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

/// The decoded `doc` section of a compiled package (plan-09-doc.md §5). Empty
/// when the package was built without any exported `DOC` blocks.
#[derive(Clone, Default)]
pub struct PackageDocs {
    pub package: Option<PackageDocEntry>,
    pub decls: Vec<DeclDocEntry>,
}

impl PackageDocs {
    pub fn is_empty(&self) -> bool {
        self.package.is_none() && self.decls.is_empty()
    }
}

#[derive(Clone)]
pub struct PackageDocEntry {
    pub name: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind`.
    pub desc: Vec<(u8, String)>,
    pub deprecated: Option<String>,
}

#[derive(Clone)]
pub struct DeclDocEntry {
    /// One of `func`, `sub`, `type`, `union`, `enum`.
    pub kind: String,
    pub name: String,
    pub signature: String,
    /// `GROUP` name (FUNC/SUB), or empty.
    pub group: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind`.
    pub desc: Vec<(u8, String)>,
    pub args: Vec<(String, String)>,
    pub props: Vec<(String, String)>,
    pub ret: String,
    pub errors: Vec<(String, String)>,
    pub example: String,
    pub internal: bool,
    pub deprecated: Option<String>,
}

const DOC_KIND_FUNC: u16 = 0;
const DOC_KIND_SUB: u16 = 1;
const DOC_KIND_TYPE: u16 = 2;
const DOC_KIND_UNION: u16 = 3;
const DOC_KIND_ENUM: u16 = 4;

/// Read the optional `doc` section from a compiled `.mfp` package. Returns an
/// empty [`PackageDocs`] when the package carries no documentation.
pub fn read_package_docs(path: &Path) -> Result<PackageDocs, String> {
    let package = read_package_binary_repr(path)?;
    Ok(package.project.docs)
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

struct MfpContainer<'a> {
    identity: MfpIdentity,
    binary_repr: &'a [u8],
}

struct MfpIdentity {
    name: String,
    ident: String,
    version: String,
    ident_key: String,
    signing_key: String,
}

struct DecodedExport {
    name: u32,
    kind: BinaryReprExportKind,
    function_id: u32,
}

struct AbiSerializer<'a> {
    strings: &'a [String],
    types: &'a TypeTable,
    constants: &'a ConstPool,
    bytes: Vec<u8>,
    type_refs: HashMap<u32, u32>,
    next_ref: u32,
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
    /// Optional documentation surface emitted as the `doc` section
    /// (plan-09-doc.md §5). Empty for projects without exported `DOC` blocks.
    docs: PackageDocs,
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

struct Section {
    id: u16,
    data: Vec<u8>,
}
