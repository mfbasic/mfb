use crate::builtins;
use crate::ir::{IrFunction, IrOp, IrProject, IrType, IrValue};
// plan-46-B: the `.mfp` locator table reuses the manifest's `Libc`/`LibType`
// vocabulary end to end, so manifest → table → wire → resolver share one set of
// types with no conversion layer between them to get wrong.
use crate::manifest::libraries::{LibType, Libc};
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
use sections::*;
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
/// Optional native-library locator table (plan-46-B §4.1). Emitted only for a
/// binding package that declares a `LINK` block; the container's optional flag
/// bit 0 ("contains native LINK metadata") is set alongside it. This lights up
/// the id the format reserved for exactly this purpose.
const SECTION_NATIVE_LIBRARY_TABLE: u16 = 10;
const SECTION_RESOURCE_TABLE: u16 = 11;
/// Optional documentation section (plan-09-doc.md §5). Self-describing and
/// length-prefixed; a consumer that does not understand it skips it entirely.
/// Ids 12-14 are reserved by the format for DEBUG_INFO/SOURCE_MAP/AUDIT_INFO,
/// so the doc table takes the next free id past the IR section.
const SECTION_DOC_TABLE: u16 = 17;
/// Optional human-facing package metadata (plan-61-D).
///
/// Named `PACKAGE_META` rather than `DESCRIPTION` so `license`/`keywords` can
/// join it later without consuming another section id. Self-contained and
/// length-prefixed like the DOC section: it does **not** intern into the string
/// pool, so it can be parsed without section 2.
///
/// **Never put security-relevant data here.** The format has no
/// "critical section" marker, so a reader that predates this section accepts a
/// package carrying it and silently ignores the contents. That is exactly right
/// for a description — a missing one is cosmetic — and exactly wrong for
/// anything a consumer must not miss.
const SECTION_PACKAGE_META: u16 = 18;
/// Field ids within section 18. Unknown ids are **skipped**, not rejected, so a
/// later field is additive within the section just as the section itself is
/// additive within the container.
const PACKAGE_META_FIELD_DESCRIPTION: u16 = 1;
const SECTION_ABI_INDEX: u16 = 15;
/// Structured Binary Representation payload section. Replaces the old flat code section as
/// the carrier of function bodies; see `crate::ir::encode_binary_repr`.
const SECTION_BINARY_REPR: u16 = 16;

/// MFPC container major version. Bumped to 2 for the clean break to the
/// structured Binary Representation payload — the reader rejects the old flat (v1) layout.
const MFPC_MAJOR_VERSION: u16 = 2;

/// ABI signature-hash input format.
///
/// bug-277 moved kind-11 (`STATE`) composites from opaque to structural hashing,
/// which shifts the `sigHash` of a stateful export — but deliberately did NOT bump
/// this. The gate in `read_abi_index` guards the section's *wire encoding*, which
/// that change leaves untouched; bumping it would reject every previously-built
/// `.mfp` wholesale, including the overwhelming majority that export no `STATE`
/// type at all. A package that does carry a stale kind-11 hash is already rejected
/// precisely, per symbol, by `validate_abi_index` recomputing it from the function
/// table. Bump this only for an actual ABI_INDEX layout change.
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
// `Money` (plan-29-B): an 8-byte base-10 fixed-point scalar. It takes the last
// freed low primitive slot, id 9 (the removed `TerminalSize`) — a primitive, so
// it belongs in the low range, not the high reserved handle range.
pub(crate) const TYPE_MONEY: u32 = 9;
// `Scalar` (plan-41-B): a 4-byte 32-bit Unicode scalar primitive. It takes id 10,
// the first slot past the previous `FIRST_TABLE_TYPE_ID`. Because assigning a new
// primitive id forces a one-time renumber of every table-type wire id (they start
// at `FIRST_TABLE_TYPE_ID`), and that cost is identical no matter how far the base
// moves, we push the base to 20 and RESERVE ids 11–19 for future primitives. The
// next primitive claims a reserved id (fill from 11) as a purely additive edit —
// no second renumber, no second golden regeneration. Reserved ids stay unmapped
// (no name→id entry, no `primitive_type_name` arm); decoding one is an error.
pub(crate) const TYPE_SCALAR: u32 = 10;
// `term::` builtin record types live in the high reserved id range alongside the
// handle types (File/Socket/Listener), not the low primitive range: ids at/above
// `FIRST_TABLE_TYPE_ID` (20) would collide with per-package user/table type ids,
// silently hijacking another package's first table type in the signature hash.
pub(crate) const TYPE_FILE_HANDLE: u32 = 0xffff_ff00;
pub(crate) const TYPE_SOCKET_HANDLE: u32 = 0xffff_feff;
pub(crate) const TYPE_LISTENER_HANDLE: u32 = 0xffff_fefe;
pub(crate) const TYPE_TERM_COLOR: u32 = 0xffff_fefd;
pub(crate) const TYPE_TERM_SIZE: u32 = 0xffff_fefc;
// First wire id for per-package table (record/union/enum) types. Bumped 10 → 20
// by plan-41-B; ids 11–19 are the reserved primitive band (see `TYPE_SCALAR`).
const FIRST_TABLE_TYPE_ID: u32 = 20;

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
    /// The `project.json` `description` (plan-61-D). Empty when the manifest
    /// declares none, in which case **section 18 is not emitted at all** — an
    /// empty section would change the bytes of every package that has no
    /// description, which is precisely what this design avoids.
    pub description: String,
    pub dependencies: Vec<BinaryReprDependency>,
    /// Native `LINK` library locators (plan-46-B). Empty for every non-binding
    /// package, in which case section 10 is not emitted and container flag bit 0
    /// stays clear.
    pub native_libraries: NativeLibraryTable,
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
            description: String::new(),
            dependencies: Vec::new(),
            native_libraries: NativeLibraryTable::default(),
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

/// The decoded `NATIVE_LIBRARY_TABLE` (section id 10) of a compiled package
/// (plan-46-B §4.1): where to find each logical `LINK` library per platform.
///
/// Empty for every package with no `LINK` block, in which case the section is not
/// emitted at all and the `.mfp` is byte-identical to a pre-plan-46 build.
///
/// The locator reuses [`crate::manifest::libraries`]'s `Libc`/`LibType` vocabulary
/// deliberately: the same types flow manifest → table → `.mfp` → resolver, so
/// there is no conversion layer between representations to get wrong.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeLibraryTable {
    /// Sorted by `logical`, so the encoding is deterministic — the repo holds a
    /// byte-identical self-diff gate.
    pub entries: Vec<NativeLibraryEntry>,
}

impl NativeLibraryTable {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The locators declared for `logical`, or `None` when the table does not
    /// carry that library.
    pub fn locators(&self, logical: &str) -> Option<&[NativeLibraryLocator]> {
        self.entries
            .iter()
            .find(|entry| entry.logical == logical)
            .map(|entry| entry.locators.as_slice())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLibraryEntry {
    /// The logical name from `LINK "<name>"`.
    pub logical: String,
    pub locators: Vec<NativeLibraryLocator>,
}

/// One platform locator, as carried in the `.mfp`.
///
/// Mirrors [`crate::manifest::libraries::LibraryLocator`] plus the build-time
/// `hash`, which is present **iff** `lib_type` is `Vendor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLibraryLocator {
    pub os: String,
    /// `None` = any arch.
    pub arch: Option<String>,
    /// `None` = any libc.
    pub libc: Option<crate::manifest::libraries::Libc>,
    pub lib_type: crate::manifest::libraries::LibType,
    /// A bare filename — the `vendor/` prefix is never encoded. It is a fixed,
    /// known location both sides derive; storing it would be redundant data that
    /// could disagree with the rule.
    pub source: String,
    /// sha256 of `<project root>/vendor/<source>`, present iff `lib_type` is
    /// `Vendor`.
    pub hash: Option<[u8; 32]>,
}

/// Wire encoding of the `libc` axis (plan-46-B §4.1).
const WIRE_LIBC_UNSPECIFIED: u8 = 0;
const WIRE_LIBC_GLIBC: u8 = 1;
const WIRE_LIBC_MUSL: u8 = 2;
/// Wire encoding of the `type` axis.
const WIRE_LIB_TYPE_SYSTEM: u8 = 0;
const WIRE_LIB_TYPE_VENDOR: u8 = 1;
/// Byte length of a locator's sha256.
const NATIVE_LIBRARY_HASH_LEN: usize = 32;

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

/// Read the optional `NATIVE_LIBRARY_TABLE` (section id 10) from a compiled
/// `.mfp` package, alongside the package's own name (plan-46-C).
///
/// The name is the locator's **declaring unit** — the prefix a `vendor` locator's
/// file is copied and `dlopen`ed under (plan-46-D §4.5) — so it must come from
/// the package itself, not from the filename on disk.
///
/// Returns an empty table for a package with no `LINK` block, which is every
/// non-binding package.
pub fn read_package_native_libraries(path: &Path) -> Result<(String, NativeLibraryTable), String> {
    let package = read_package_binary_repr(path)?;
    let name = package
        .project
        .strings
        .values
        .get(package.project.manifest.package_name as usize)
        .cloned()
        .unwrap_or_default();
    Ok((name, package.project.native_libraries))
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

/// [`read_package_info`] for a `.mfp` already held in memory, so a caller with a
/// downloaded blob never has to stage it to a predictable path on disk to read it.
pub fn package_info_from_mfp(bytes: &[u8]) -> Result<BinaryReprPackageInfo, String> {
    let container = mfp_binary_repr_payload(bytes)?;
    let package = read_binary_repr_package(container.binary_repr)?;
    validate_container_manifest_identity(&container.identity, &package)?;
    package_info(&package)
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
    /// Current composite-type recursion depth, capped at `MAX_TYPE_GRAPH_DEPTH`
    /// so an untrusted deep-but-acyclic type chain cannot overflow the stack
    /// (bug-153). `type_refs` only grows, so it cannot serve as a depth gauge.
    depth: usize,
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
    /// Optional native `LINK` locator table emitted as section 10 (plan-46-B).
    /// Empty for every package without a `LINK` block.
    native_libraries: NativeLibraryTable,
    /// The `description` carried in section 18 (plan-61-D). Empty when the
    /// package declares none, in which case the section is not emitted.
    description: String,
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
    /// Redundant table counts the writer emits and `read_binary_repr_package`
    /// cross-validates against the decoded tables (bug-282 B4). They were
    /// previously decoded into `_`-prefixed locals and discarded, so a crafted
    /// manifest could claim any counts it liked.
    dependency_count: u32,
    export_count: u32,
}

#[derive(Clone)]
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
