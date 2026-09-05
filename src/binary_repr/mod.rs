use crate::codegen::builtins;
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

// bug-340 B8: the `.mfp` container is the wire format `binary_repr` owns. The
// manifest-layer header reader (`manifest::package::read_mfp_header`) shares its
// magic and signature-header rule from here rather than re-implementing them.
// (The two full decoders are deliberately NOT merged — the manifest reader
// additionally enforces per-field byte limits, UTF-8, required-non-empty, and
// `validate_package_name`, and returns fields this identity/payload decoder omits;
// folding them would drop those trust-boundary guards. See the bug-340 B8 note.)
pub(crate) use reader::validate_mfp_signature_header;
use writer::*;

// Section ids are wire format and frozen; the values are declared here in
// numeric order (the [`SectionKind`] enum in reader.rs is the typed handle that
// fetches them). Ids 12-14 are reserved by the format for
// DEBUG_INFO/SOURCE_MAP/AUDIT_INFO, and ids 9, 19 are unassigned gaps.
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
const SECTION_ABI_INDEX: u16 = 15;
/// Structured Binary Representation payload section. Replaces the old flat code section as
/// the carrier of function bodies; see `crate::ir::encode_binary_repr`.
const SECTION_BINARY_REPR: u16 = 16;
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

/// MFPC container major version. Bumped to 2 for the clean break to the
/// structured Binary Representation payload — the reader rejects the old flat (v1) layout.
const MFPC_MAJOR_VERSION: u16 = 2;

/// The 8-byte `.mfp` container magic (plan-23 §4). The single home shared by this
/// crate's `mfp_binary_repr_payload` and the manifest layer's `read_mfp_header`,
/// which previously each defined their own copy (bug-340 B8).
pub(crate) const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];

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
// `AttributedString` (plan-89-A): an opaque, value-semantic built-in wrapping a
// visible `String` plus an attribute overlay. It is a primitive-like hardcoded
// type (modeled on `Error`), so it claims the first reserved primitive-band id
// (11) as a purely additive edit — no table-id renumber (see `TYPE_SCALAR`).
pub(crate) const TYPE_ATTRIBUTED_STRING: u32 = 11;
// `term::` builtin record types live in the high reserved id range alongside the
// handle types (File/Socket/Listener), not the low primitive range: ids at/above
// `FIRST_TABLE_TYPE_ID` (20) would collide with per-package user/table type ids,
// silently hijacking another package's first table type in the signature hash.
pub(crate) const TYPE_FILE_HANDLE: u32 = 0xffff_ff00;
pub(crate) const TYPE_SOCKET_HANDLE: u32 = 0xffff_feff;
pub(crate) const TYPE_LISTENER_HANDLE: u32 = 0xffff_fefe;
/// RETIRED by plan-122-F, and deliberately NOT recycled.
///
/// `term::TermColor` no longer exists — the colour members speak `color::Color` —
/// so no encoder emits this id. It stays reserved because a `.mfp` published
/// before that change still carries it, and `binary_repr::reader` still decodes it
/// to a recognizable name rather than failing opaquely. Assigning `0xffff_fefd` to
/// a new type would silently mis-decode those packages.
pub(crate) const TYPE_TERM_COLOR: u32 = 0xffff_fefd;
pub(crate) const TYPE_TERM_SIZE: u32 = 0xffff_fefc;
// First wire id for per-package table (record/union/enum) types. Bumped 10 → 20
// by plan-41-B; ids 11–19 are the reserved primitive band (see `TYPE_SCALAR`).
const FIRST_TABLE_TYPE_ID: u32 = 20;

// bug-390: a type-table entry kind for a reference to a type owned by an imported
// dependency package. Unlike an inline definition (kinds 1/2/3) it carries no
// fields of its own — its payload is `[u16 underlying-export-kind][32-byte owning
// ABI hash]`, its `name` is the type's original name and its `owner_package` the
// declaring dependency. This lets a package's exported API name a dependency's
// type without degrading it to a zero-field record (the old `_` fallback, which
// failed downstream with `truncated binary representation`). Serializing it by the
// owning package's ABI hash (never re-walking absent fields) gives the "original
// identity, no re-mangle" property, so the same `pA::A` surfaced through two
// intermediaries hashes identically and unifies at the consumer.
const FOREIGN_TYPE_KIND: u16 = 12;

const FUNCTION_BINARY_REPR: u16 = 1;

const FUNCTION_FLAG_ISOLATED: u16 = 1 << 2;
const FUNCTION_FLAG_PRIVATE: u16 = 1 << 1;
const FUNCTION_FLAG_SUB: u16 = 1 << 3;
const FUNCTION_FLAG_RETURNS_NOTHING: u16 = 1 << 5;

// ===== Public API: exported types =====
// The decoded surfaces `read_package_*` return and the metadata the builders
// take. The public entry-point functions follow, after all the types
// (bug-335 B6); the module's internal wire structs come last.

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
    /// plan-111-B: typed. The `.mfp` wire still carries the spelling — this is
    /// the DECODED view, and `binary_repr::builder` (the `.mfp` codec, boundary
    /// #4) is where the type table's text becomes a type, once, instead of at
    /// each consumer.
    pub return_type: crate::types::ParameterType,
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
    pub type_: crate::types::ParameterType,
    pub has_default: bool,
}

#[derive(Clone)]
pub struct BinaryReprTypeExport {
    pub name: String,
    pub kind: BinaryReprExportKind,
    pub fields: Vec<BinaryReprTypeField>,
    pub variants: Vec<BinaryReprTypeVariant>,
    pub members: Vec<String>,
    /// bug-390: `Some(owning_package)` when this export is a type re-exported
    /// from a dependency (a `FOREIGN_TYPE_KIND` table entry) rather than defined
    /// here. `read_package_type_exports` fills in the real fields/variants from
    /// the owner's sibling `.mfp`; `None` for a type this package defines.
    pub foreign_owner: Option<String>,
}

#[derive(Clone)]
pub struct BinaryReprTypeField {
    pub name: String,
    pub type_: String,
    /// The field's declared visibility, as the table records it. Not read by the
    /// compiler since plan-107-D: member visibility is enforced per TYPE by
    /// `ir::verify` (`type_decl_info`), and an imported record's fields are
    /// presented through `ir::ImportedTypeField` without it. Kept so the decoded
    /// row is not a lossy copy of the table.
    #[allow(dead_code)]
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

/// One resource type contributed by an imported package's `RESOURCE_TABLE`
/// (the return element of [`read_package_resources`]).
///
/// `native` distinguishes native (`LINK`) resources from standard ones; it is
/// read when the former source checker registers an imported package's resource types
/// (every field is consumed there), so no field is dead.
pub struct BinaryReprResourceExport {
    pub type_name: String,
    /// Resolved close-op name (`fs.close`/`net.close` for built-ins, or the
    /// declaring package's close function name). `None` when the close function
    /// id cannot be resolved.
    pub close_function: Option<String>,
    pub sendable: bool,
    /// Whether the close op can fail, as the table records it. Not read by the
    /// compiler since plan-107-D: drop-time cleanup derives the same fact from
    /// the close wrapper's `SUCCESS ON` (`lower_link::native_resources`). Kept so
    /// the decoded row is not a lossy copy of the table.
    #[allow(dead_code)]
    pub close_may_fail: bool,
    pub native: bool,
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
const DOC_KIND_RESOURCE: u16 = 5;

// ===== Public API: build + read entry points =====

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
// plan-110-E: the standard stream close op. The NUMERIC id is format state and
// must not change -- existing `.mfp` files reference it -- but the resource it
// names moved from `net` to `tcp` when net's stream surface was removed.
pub(crate) const BUILTIN_STREAM_CLOSE_FUNCTION_ID: u32 = 0xffff_feff;
/// "Resolve this built-in resource's close op from the registry, by the entry's
/// own type name" (bug-464 fallout).
///
/// The two sentinels above name ONE resource each, and the writer's table was a
/// hardcoded three-name allowlist to match — `fs.File`, `tcp.Socket`,
/// `tcp.Listener`. Every other built-in resource (`udp::Socket`, the `tls` pair,
/// `process::Process`, the audio handles, `canvas::Image`) therefore got no
/// `RESOURCE_TABLE` entry at all, and a package exporting one failed to build
/// with an opaque `truncated binary representation`. A resource entry already
/// carries its `type_id`, so the close op is derivable rather than needing a new
/// sentinel per type; this id says "do that".
///
/// The two legacy sentinels are still WRITTEN for their three types so existing
/// `.mfp` bytes are unchanged, and still decoded so older packages keep loading.
pub(crate) const BUILTIN_RESOURCE_CLOSE_BY_TYPE: u32 = 0xffff_fefe;

pub fn read_package_exports(path: &Path) -> Result<Vec<BinaryReprExport>, String> {
    let package = read_package_binary_repr(path)?;
    package_exports(&package).map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

/// One installed `.mfp`, decoded ONCE, with each section an importing build needs
/// read off that single decode (plan-105-A).
///
/// The driver derives three views of a dependency — its function signatures, its
/// exported type layouts, and its resource close ops. Each used to come from its
/// own top-level reader, so every dependency was read off disk and decoded three
/// times over, and the three had to stay in lockstep for the build to see a
/// coherent picture of a package (`planning/Compiler Pipeline.md:58`).
///
/// Each accessor still returns a `Result` so a caller keeps today's *per-section*
/// error recovery: a package whose resource table is unreadable can still
/// contribute its function signatures, exactly as when the three readers were
/// independent. What is shared is the decode, not the failure.
pub struct BinaryReprPackageDecode<'a> {
    package: PackageBinaryRepr,
    path: &'a Path,
}

impl<'a> BinaryReprPackageDecode<'a> {
    pub fn read(path: &'a Path) -> Result<Self, String> {
        Ok(Self {
            package: read_package_binary_repr(path)?,
            path,
        })
    }

    fn fail(&self, err: String) -> String {
        format!("failed to read '{}': {err}", self.path.display())
    }

    /// The package's manifest name. Equals the container header's name by
    /// construction: [`read_package_binary_repr`] rejects a file whose header
    /// identity disagrees with the binary-representation manifest
    /// (`validate_container_manifest_identity`).
    pub fn name(&self) -> Result<String, String> {
        package_info(&self.package)
            .map(|info| info.manifest_name)
            .map_err(|err| self.fail(err))
    }

    pub fn exports(&self) -> Result<Vec<BinaryReprExport>, String> {
        package_exports(&self.package).map_err(|err| self.fail(err))
    }

    pub fn type_exports(&self) -> Result<Vec<BinaryReprTypeExport>, String> {
        resolve_package_type_exports(&self.package, self.path, 0)
    }

    pub fn resources(&self) -> Result<Vec<BinaryReprResourceExport>, String> {
        package_resource_exports(&self.package).map_err(|err| self.fail(err))
    }
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
    read_package_type_exports_resolved(path, 0)
}

/// bug-390: a foreign type this package references, carrying the owning package
/// name, the type's original name, and the owning package's ABI hash for it.
/// Used by the consumer build to reject an ABI-incompatible owner (two
/// intermediaries built against different versions of the shared dependency, or
/// an intermediary built against a different owner than the consumer resolves).
pub struct BinaryReprForeignTypeRef {
    pub name: String,
    pub owner: String,
    pub abi_hash: [u8; ABI_HASH_LEN],
}

pub fn read_package_foreign_type_refs(
    path: &Path,
) -> Result<Vec<BinaryReprForeignTypeRef>, String> {
    let package = read_package_binary_repr(path)?;
    let strings = &package.project.strings.values;
    let mut refs = Vec::new();
    for entry in &package.project.types.entries {
        if entry.kind != FOREIGN_TYPE_KIND {
            continue;
        }
        let hash_slice = entry
            .payload
            .get(2..2 + ABI_HASH_LEN)
            .ok_or("truncated binary representation")?;
        let mut abi_hash = [0u8; ABI_HASH_LEN];
        abi_hash.copy_from_slice(hash_slice);
        refs.push(BinaryReprForeignTypeRef {
            name: string_at(strings, entry.name)?.to_string(),
            owner: string_at(strings, entry.owner_package)?.to_string(),
            abi_hash,
        });
    }
    Ok(refs)
}

/// bug-390: the ABI hash the owning package publishes for each of its own
/// exported types, keyed by type name — used to check a foreign reference's
/// stored hash against the owner the consumer actually resolves.
pub fn read_package_type_export_hashes(
    path: &Path,
) -> Result<HashMap<String, [u8; ABI_HASH_LEN]>, String> {
    let package = read_package_binary_repr(path)?;
    let strings = &package.project.strings.values;
    let mut hashes = HashMap::new();
    for export in &package.project.abi.exports {
        if !matches!(
            export.kind,
            BinaryReprExportKind::Type | BinaryReprExportKind::Union | BinaryReprExportKind::Enum
        ) {
            continue;
        }
        hashes.insert(
            string_at(strings, export.name)?.to_string(),
            export.sig_hash,
        );
    }
    Ok(hashes)
}

/// bug-390: resolve any re-exported foreign type (`foreign_owner: Some`) to the
/// owning dependency's real definition, read from its sibling `.mfp` in the same
/// `packages/` directory. This delivers true namespace re-export — an importer of
/// the intermediary package sees the dependency's type with fields/variants
/// intact and under the owning package's identity, idempotently however many
/// intermediaries surface it. A package with no foreign type reads no siblings,
/// so existing outputs are untouched. The depth cap breaks a pathological
/// re-export cycle (packages form a DAG, so a real chain is shallow).
fn read_package_type_exports_resolved(
    path: &Path,
    depth: usize,
) -> Result<Vec<BinaryReprTypeExport>, String> {
    let package = read_package_binary_repr(path)?;
    resolve_package_type_exports(&package, path, depth)
}

/// [`read_package_type_exports_resolved`] on an ALREADY-decoded package, so a
/// caller that needs several sections of one `.mfp` decodes the file once
/// (plan-105-A) instead of once per section. `path` is still required: resolving
/// a re-exported foreign type reads the *owning* package from the same directory.
fn resolve_package_type_exports(
    package: &PackageBinaryRepr,
    path: &Path,
    depth: usize,
) -> Result<Vec<BinaryReprTypeExport>, String> {
    const MAX_REEXPORT_DEPTH: usize = 64;
    let mut exports = package_type_exports(package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    if !exports.iter().any(|export| export.foreign_owner.is_some()) {
        return Ok(exports);
    }
    if depth >= MAX_REEXPORT_DEPTH {
        return Err(format!(
            "package re-export chain exceeds {MAX_REEXPORT_DEPTH} levels at '{}'",
            path.display()
        ));
    }
    let packages_dir = path.parent();
    // Each owning dependency's fully-resolved export list, read at most once and
    // reused for both the foreign-marker fill and the transitive closure below.
    let mut owner_pool: HashMap<String, Vec<BinaryReprTypeExport>> = HashMap::new();
    for export in &mut exports {
        let Some(owner) = export.foreign_owner.clone() else {
            continue;
        };
        let Some(dir) = packages_dir else {
            continue;
        };
        // bug-395: `owner` is decoded verbatim from an untrusted `.mfp` and here
        // becomes a filename joined onto the packages directory. A hostile owner
        // like `../../etc/foo` or an absolute path would walk out of the
        // directory — an existence oracle for any `*.mfp` on the victim's disk,
        // and an attacker-triggered read of it. Re-validate it as a bare package
        // name (the rule every `packages/<name>.mfp` obeys) before the join, just
        // as the sibling native-library `source` locator does (sections.rs).
        crate::manifest::package::validate_package_name(&owner)?;
        let owner_path = dir.join(format!("{owner}.mfp"));
        if !owner_path.is_file() {
            // The owner is not installed alongside the intermediary; the type's
            // name still resolves, but its fields cannot be filled in here.
            continue;
        }
        if !owner_pool.contains_key(&owner) {
            let owner_exports = read_package_type_exports_resolved(&owner_path, depth + 1)?;
            owner_pool.insert(owner.clone(), owner_exports);
        }
        if let Some(def) = owner_pool[&owner]
            .iter()
            .find(|candidate| candidate.name == export.name)
        {
            export.kind = def.kind;
            export.fields = def.fields.clone();
            export.variants = def.variants.clone();
            export.members = def.members.clone();
        }
    }
    // bug-435: the fill above resolves each re-exported type's own
    // fields/variants, but the *other* user types those fields/variants
    // reference (a record field's type, a union variant field's type) are not
    // themselves named in this package's ABI, so they were dropped — leaving the
    // package non-self-contained and any importer rejecting it with
    // `PACKAGE_INVALID: references unknown type`. Walk the transitive closure:
    // every `Type::User` reachable from a re-exported type is resolved from the
    // same owner's export list (already fully resolved above) and appended. The
    // owner's resolved list is itself closed, so a type it re-exports from a
    // deeper dependency is present there too. Cycle-guarded by `seen` (a
    // self-referential `List OF Node` terminates).
    let mut seen: HashSet<String> = exports.iter().map(|e| e.name.clone()).collect();
    let mut queue: Vec<(String, String)> = Vec::new();
    for export in &exports {
        if let Some(owner) = &export.foreign_owner {
            enqueue_referenced_types(export, owner, &mut queue);
        }
    }
    while let Some((name, owner)) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(owner_exports) = owner_pool.get(&owner) else {
            continue;
        };
        let Some(def) = owner_exports
            .iter()
            .find(|candidate| candidate.name == name)
        else {
            continue;
        };
        let def = def.clone();
        enqueue_referenced_types(&def, &owner, &mut queue);
        exports.push(def);
    }
    Ok(exports)
}

/// bug-435: collect the user-type names referenced by a re-exported type's own
/// fields and its union variants' fields, pairing each with the owner it must be
/// resolved from. A field's `type_` is a rendered type string (`Meta`,
/// `List OF Node`, `Map OF String TO Meta`, …); every user type in it is an
/// identifier, so pulling the identifier tokens over-approximates the referenced
/// names — the caller keeps only those that resolve to an actual export in the
/// owner's list, which naturally discards built-in tokens (`List`, `String`, …).
fn enqueue_referenced_types(
    export: &BinaryReprTypeExport,
    owner: &str,
    queue: &mut Vec<(String, String)>,
) {
    for field in &export.fields {
        push_type_identifiers(&field.type_, owner, queue);
    }
    for variant in &export.variants {
        // A union variant *is* a record type the owner exports standalone (e.g.
        // `EXPORT TYPE Box` backing `UNION Node { Box }`). Native codegen lays the
        // union's block out by inlining each variant's record, so it needs that
        // record registered in `record_fields`, not only the variant's field list
        // — pull the variant record itself into the closure, not just its fields.
        queue.push((variant.name.clone(), owner.to_string()));
        for field in &variant.fields {
            push_type_identifiers(&field.type_, owner, queue);
        }
    }
}

/// Push each maximal identifier substring of a rendered type string onto the
/// resolution queue, tagged with the owner package to resolve it from.
fn push_type_identifiers(rendered: &str, owner: &str, queue: &mut Vec<(String, String)>) {
    let mut current = String::new();
    for ch in rendered.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            queue.push((std::mem::take(&mut current), owner.to_string()));
        }
    }
    if !current.is_empty() {
        queue.push((current, owner.to_string()));
    }
}

/// Decode an imported package's `RESOURCE_TABLE` so the importer can register
/// the package's resource types (recognition, sendability, and close op) instead
/// of relying on hardcoded knowledge of the standard built-ins.
pub fn read_package_resources(path: &Path) -> Result<Vec<BinaryReprResourceExport>, String> {
    let package = read_package_binary_repr(path)?;
    package_resource_exports(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

/// The content-addressed identity prefix `merge_packages` qualifies this
/// package's merged symbols with (`<id>.<package>.<symbol>`).
///
/// bug-377: a consumer that resolves a package symbol by name *after* the merge
/// — the resource close op in `code::validation` — has to spell it the same way
/// [`crate::ir::prefix_package_symbols`] did, or the lookup silently misses and
/// the resource is never closed.
pub fn read_package_identity_id(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let container = mfp_binary_repr_payload(&bytes)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    Ok(package_identity_id(
        &container.identity,
        container.binary_repr,
    ))
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

pub(crate) fn build_binary_repr_bytes(
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

// ===== Internal wire types =====

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
    /// bug-390: imported dependency types (by original name) that this build may
    /// surface in its own exported API. Populated on the *write* path before
    /// lowering (from each dependency's ABI type exports); empty on the read path.
    /// `TypeTable::type_id`'s fallback consults this to emit a `FOREIGN_TYPE_KIND`
    /// reference instead of an empty-record placeholder.
    foreign_types: HashMap<String, ForeignTypeRef>,
}

/// bug-390: the identity of a dependency's exported type, carried so a package
/// that names it in its own public API can re-export it by the owning package's
/// original ABI identity. `abi_hash` is the owning package's `type_sig_hash` for
/// this type — the load-bearing field a name alone cannot supply.
#[derive(Clone)]
struct ForeignTypeRef {
    package: String,
    export_kind: BinaryReprExportKind,
    abi_hash: [u8; ABI_HASH_LEN],
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
