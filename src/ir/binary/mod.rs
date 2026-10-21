//! Binary Representation (structured) encode/decode.
//!
//! The package payload is a faithful, versioned binary serialization of the
//! compiler's IR (`IrProject` / `IrFunction` / `IrOp` / `IrValue` / `IrType`).
//! It is *not* a flat opcode stream: control flow stays nested (regions with
//! explicit ends) and expressions stay as trees, exactly as in memory. The
//! in-memory IR is free to change behind this format; this encoding is the
//! stable contract.
//!
//! The format is self-contained: strings are inline length-prefixed, integers
//! are little-endian. There is no separate interned pool here — the `.mfp`
//! container's tables (manifest/ABI/import/export) are kept and derived
//! alongside this payload by `binary_repr.rs`, but a function body is faithfully
//! reconstructable from this payload alone.

use super::*;
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;

mod link_codec;
mod op_codec;
mod value_codec;
use link_codec::{
    decode_cstructs, decode_link_function, decode_link_state_trailer, encode_link_function,
    encode_link_state_trailer,
};
use op_codec::{decode_op, encode_op};
use value_codec::{decode_value, encode_value, get_loc, put_loc};

/// Magic bytes prefixing a Binary Representation payload.
pub const BINARY_REPR_MAGIC: &[u8; 4] = b"MFBR";
/// Binary Representation format version. Bump on any incompatible change to the encoding.
/// Version 2 adds per-node source locations (`loc` on Call/CallResult/Binary/Unary/For)
/// and a per-function source `file`, backing read-only `Error.source` / `ErrorLoc`.
/// Version 3 (plan-20-A) extends spans to the full diagnostic vocabulary: every
/// op, match case, and declaration (function/param/type/field/variant/binding)
/// carries a trailing `loc`, so the IR-level semantic checker can report at the
/// same source line the AST checker does.
/// Version 4 (plan-20-Z) adds the declaration-fidelity fields the relocated
/// rules need: `Bind` ops and top-level bindings carry `explicit_type`
/// (whether the type came from a source `AS T` annotation), and types and
/// top-level bindings carry their declaring source `file`. Version 4 also
/// serializes each function's `resource_owners` map (escape-analysis ownership
/// decisions), which a decoded package has no AST to reconstruct.
/// Version 5 adds the `CSTRUCT` table to the `LINK` trailer, replaces each ABI
/// slot's `is_out` bool with an `AbiDirection` byte (`In`/`Out`/`InOut`), gives
/// `IrLinkExpr::Var` a slot-name payload, and carries each function's `bind_in`
/// table (plan-50-C). Slots are positional with no per-field tag, so every one of
/// those is a hard format break; they ride one bump rather than four.
///
/// plan-53 carries a native LINK function's `return_state_type` and `bind_state`
/// (a stateful native resource's STATE type + the OUT struct slot that populates
/// it) as an OPTIONAL APPEND-ONLY TRAILER, NOT a version bump — see
/// `encode_link_state_trailer`. A package with no stateful native resource writes
/// no trailer and is byte-identical to before; a decoder that reaches end-of-input
/// simply leaves every link function stateless. This avoids re-churning every
/// committed `.mfp` (including the hand-crafted security decode-hardening vectors,
/// which pin an exact version).
///
/// Version 7 (bug-945) renames the internal fallible result inside every inline
/// type string: `Result OF T` is now `#Result OF T`, because `Result OF T` is an
/// ordinary user template. A v6 payload's `Result OF T` would decode as that user
/// template, so v6 is refused and its packages must be rebuilt.
///
/// Version 8 (plan-180-C) adds an optional loop `target` name to `ExitLoop` and
/// `ContinueLoop` (tags 11/12), written after the loop kind. A v7 payload would
/// decode those tags without the field, so v7 is refused and its packages must
/// be rebuilt.
pub const BINARY_REPR_VERSION: u16 = 8;

// --- low-level writers -----------------------------------------------------

fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_bool(out: &mut Vec<u8>, v: bool) {
    out.push(if v { 1 } else { 0 });
}

fn put_loop_kind(out: &mut Vec<u8>, kind: LoopKind) {
    put_u8(
        out,
        match kind {
            LoopKind::For => 0,
            LoopKind::Do => 1,
            LoopKind::While => 2,
        },
    );
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn put_opt_str(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(v) => {
            put_u8(out, 1);
            put_str(out, v);
        }
        None => put_u8(out, 0),
    }
}

fn put_vec<T, F: Fn(&mut Vec<u8>, &T)>(out: &mut Vec<u8>, items: &[T], f: F) {
    put_u32(out, items.len() as u32);
    for item in items {
        f(out, item);
    }
}

fn put_opt_value(out: &mut Vec<u8>, value: &Option<IrValue>) {
    match value {
        Some(v) => {
            put_u8(out, 1);
            encode_value(out, v);
        }
        None => put_u8(out, 0),
    }
}

// --- low-level reader ------------------------------------------------------

/// Recursion cap for the Binary Representation body decoder (PKG-03). A crafted
/// package can nest expressions/statements arbitrarily deep; each level is one
/// native stack frame, so an unbounded tree overflows the stack and aborts the
/// compiler before any structural check runs. 256 is far deeper than any real
/// program yet shallow enough to never overflow the thread stack.
///
/// Forwards to the single [`super::MAX_IR_NESTING_DEPTH`] source (bug-342 A3) so
/// the decoder's cap and `ir::verify`'s cap can never diverge.
const MAX_DECODE_DEPTH: usize = super::MAX_IR_NESTING_DEPTH;

struct IrReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// Current recursion depth of the op/value/link-expr decoders. Bounded by
    /// [`MAX_DECODE_DEPTH`]; see [`IrReader::enter`].
    depth: usize,
}

impl<'a> IrReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        IrReader {
            bytes,
            pos: 0,
            depth: 0,
        }
    }

    /// Enter one recursion level, erroring past [`MAX_DECODE_DEPTH`]. The caller
    /// must pair a successful `enter` with [`IrReader::leave`] on every exit path.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DECODE_DEPTH {
            self.depth -= 1;
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED: expression/statement nesting exceeds the {MAX_DECODE_DEPTH} level decode limit"
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn need(&self, n: usize) -> Result<(), String> {
        // `checked_add` keeps the bound overflow-safe on every target (PKG-07):
        // `n` originates from an attacker `u32` (string/blob lengths), so on a
        // 32-bit host `pos + n` could wrap and spuriously pass the check.
        if self
            .pos
            .checked_add(n)
            .is_none_or(|end| end > self.bytes.len())
        {
            Err(format!(
                "Binary Representation truncated: needed {n} bytes at offset {}, have {}",
                self.pos,
                self.bytes.len()
            ))
        } else {
            Ok(())
        }
    }

    fn u8(&mut self) -> Result<u8, String> {
        self.need(1)?;
        let v = self.bytes[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.need(4)?;
        let v = u32::from_le_bytes([
            self.bytes[self.pos],
            self.bytes[self.pos + 1],
            self.bytes[self.pos + 2],
            self.bytes[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        self.need(len)?;
        let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + len])
            .map_err(|err| format!("Binary Representation: invalid UTF-8 string: {err}"))?
            .to_string();
        self.pos += len;
        Ok(s)
    }

    fn opt_string(&mut self) -> Result<Option<String>, String> {
        if self.u8()? != 0 {
            Ok(Some(self.string()?))
        } else {
            Ok(None)
        }
    }

    fn count(&mut self) -> Result<usize, String> {
        Ok(self.u32()? as usize)
    }

    /// Whether the reader has consumed all bytes. Used to keep the native `LINK`
    /// tables an optional, append-only trailer so older `version 2` packages
    /// (which lack them) still decode (plan-linker.md §12).
    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn opt_value(&mut self) -> Result<Option<IrValue>, String> {
        if self.u8()? != 0 {
            Ok(Some(decode_value(self)?))
        } else {
            Ok(None)
        }
    }
}

// --- public entry points ---------------------------------------------------

/// Serialize an `IrProject` to the versioned Binary Representation byte format.
pub fn encode_binary_repr(project: &IrProject) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(BINARY_REPR_MAGIC);
    put_u16(&mut out, BINARY_REPR_VERSION);
    encode_project(&mut out, project);
    out
}

/// Decode a Binary Representation byte payload back into an `IrProject`.
pub fn decode_binary_repr(bytes: &[u8]) -> Result<IrProject, String> {
    let mut r = IrReader::new(bytes);
    r.need(4)?;
    if &bytes[0..4] != BINARY_REPR_MAGIC {
        return Err("Binary Representation: bad magic (expected MFBR)".to_string());
    }
    r.pos = 4;
    let version = r.u16()?;
    if version != BINARY_REPR_VERSION {
        return Err(format!(
            "Binary Representation version {version} unsupported (expected {BINARY_REPR_VERSION})"
        ));
    }
    decode_project(&mut r)
}

// --- IrProject -------------------------------------------------------------

fn encode_project(out: &mut Vec<u8>, project: &IrProject) {
    put_str(out, &project.name);
    match &project.entry {
        Some(entry) => {
            put_u8(out, 1);
            put_str(out, &entry.name);
            put_str(out, &entry.returns.name());
            put_bool(out, entry.accepts_args);
        }
        None => put_u8(out, 0),
    }
    put_vec(out, &project.bindings, encode_binding);
    put_vec(out, &project.types, encode_type);
    put_vec(out, &project.functions, encode_function);
    // Native `LINK` functions + re-export aliases so an importer can rebuild the
    // marshaling thunks and routing (plan-linker.md §12). Written only when
    // present, as an optional append-only trailer, so packages without `LINK`
    // blocks stay byte-identical to the pre-feature `version 2` encoding.
    if !project.link_functions.is_empty()
        || !project.link_aliases.is_empty()
        || !project.link_cstructs.is_empty()
    {
        put_vec(out, &project.link_functions, encode_link_function);
        put_vec(out, &project.link_aliases, |o, (alias, target)| {
            put_str(o, alias);
            put_str(o, target);
        });
        // plan-50-C: field NAMES and CTYPES only. Offsets and sizes are never on
        // the wire — they are recomputed from the ctypes at decode, so a crafted
        // package has no offset to forge.
        put_vec(out, &project.link_cstructs, |o, c| {
            put_str(o, &c.alias);
            put_str(o, &c.name);
            put_str(o, &c.maps_to.name());
            put_vec(o, &c.fields, |o, f| {
                put_str(o, &f.name);
                put_str(o, &f.ctype.name());
            });
        });
        encode_link_state_trailer(out, &project.link_functions);
    }
}

fn decode_project(r: &mut IrReader) -> Result<IrProject, String> {
    let name = r.string()?;
    let entry = if r.u8()? != 0 {
        Some(EntryPoint {
            name: r.string()?,
            returns: ParameterType::parse(&r.string()?),
            accepts_args: r.bool()?,
        })
    } else {
        None
    };
    let bindings = decode_vec(r, decode_binding)?;
    let types = decode_vec(r, decode_type)?;
    let functions = decode_vec(r, decode_function)?;
    // Native `LINK` tables are an optional append-only trailer: packages built
    // before this feature simply end after `functions` (plan-linker.md §12).
    let (link_functions, link_aliases, link_cstructs) = if r.at_end() {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        let mut link_functions = decode_vec(r, decode_link_function)?;
        let link_aliases = decode_vec(r, |r| Ok((r.string()?, r.string()?)))?;
        let link_cstructs = decode_cstructs(r)?;
        // plan-53: patch each link function's STATE from the optional trailer.
        decode_link_state_trailer(r, &mut link_functions)?;
        (link_functions, link_aliases, link_cstructs)
    };
    Ok(IrProject {
        name,
        entry,
        bindings,
        types,
        functions,
        // Native resource metadata is consumed directly from the package's
        // RESOURCE_TABLE on import; the decoded IR carries none.
        native_resources: Vec::new(),
        link_functions,
        link_cstructs,
        link_aliases,
        // Docs live in a separate optional package section, not in the decoded IR.
        docs: ProjectDocs::default(),
        // Likewise the native library locators: they are `.mfp` section 10, read
        // by the consumer's codegen straight off the package (plan-46-C), not
        // carried in the decoded IR.
        native_libraries: crate::binary_repr::NativeLibraryTable::default(),
        // Not on the wire: the CONSUMING project's project.json sets it, so a
        // decoded package must not carry (or override) a ceiling.
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
        selected_config: Default::default(),
    })
}

fn decode_vec<T, F: Fn(&mut IrReader) -> Result<T, String>>(
    r: &mut IrReader,
    f: F,
) -> Result<Vec<T>, String> {
    let n = r.count()?;
    // Never pre-allocate more slots than the remaining bytes could possibly
    // fill (PKG-05): every element consumes at least one byte on the wire, so an
    // attacker `count` of `0xFFFF_FFFF` behind a tiny body cannot request a
    // multi-gigabyte allocation. The vec still grows to the true length.
    let mut out = Vec::with_capacity(n.min(r.bytes.len().saturating_sub(r.pos)));
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

// --- IrBinding -------------------------------------------------------------

fn encode_binding(out: &mut Vec<u8>, b: &IrBinding) {
    put_str(out, &b.name);
    put_str(out, &b.visibility);
    put_bool(out, b.mutable);
    put_str(out, &b.type_.name());
    put_opt_value(out, &b.value);
    put_loc(out, b.loc);
    put_str(out, &b.file);
    put_bool(out, b.explicit_type);
}

fn decode_binding(r: &mut IrReader) -> Result<IrBinding, String> {
    Ok(IrBinding {
        name: r.string()?,
        visibility: r.string()?,
        mutable: r.bool()?,
        type_: ParameterType::parse(&r.string()?),
        value: r.opt_value()?,
        loc: get_loc(r)?,
        file: r.string()?,
        explicit_type: r.bool()?,
    })
}

// --- IrType / IrField / IrVariant / IrEnumMember ---------------------------

fn encode_field(out: &mut Vec<u8>, f: &IrField) {
    put_opt_str(out, &f.visibility);
    put_str(out, &f.name);
    put_str(out, &f.type_.name());
    put_loc(out, f.loc);
}

fn decode_field(r: &mut IrReader) -> Result<IrField, String> {
    Ok(IrField {
        visibility: r.opt_string()?,
        name: r.string()?,
        type_: ParameterType::parse(&r.string()?),
        loc: get_loc(r)?,
    })
}

fn encode_variant(out: &mut Vec<u8>, v: &IrVariant) {
    put_str(out, &v.name);
    put_vec(out, &v.fields, encode_field);
    put_loc(out, v.loc);
}

fn decode_variant(r: &mut IrReader) -> Result<IrVariant, String> {
    let name = r.string()?;
    Ok(IrVariant {
        // bug-657: rebuilt from the encoded name rather than added to the wire, so the
        // format is unchanged. This file is boundary #2 — "IR wire/JSON decode reads
        // types as text" — and `lower_variant` derives `name` from this same type, so
        // the round trip is exact.
        type_: ParameterType::parse(&name),
        name,
        fields: decode_vec(r, decode_field)?,
        loc: get_loc(r)?,
    })
}

fn encode_type(out: &mut Vec<u8>, t: &IrType) {
    put_str(out, &t.kind);
    put_str(out, &t.visibility);
    put_str(out, &t.name);
    put_vec(out, &t.fields, encode_field);
    put_vec(out, &t.includes, |o, s| put_str(o, s));
    put_vec(out, &t.variants, encode_variant);
    put_vec(out, &t.members, |o, m| put_str(o, &m.name));
    put_loc(out, t.loc);
    put_str(out, &t.file);
}

fn decode_type(r: &mut IrReader) -> Result<IrType, String> {
    Ok(IrType {
        kind: r.string()?,
        visibility: r.string()?,
        name: r.string()?,
        fields: decode_vec(r, decode_field)?,
        includes: decode_vec(r, |r| r.string())?,
        variants: decode_vec(r, decode_variant)?,
        members: decode_vec(r, |r| Ok(IrEnumMember { name: r.string()? }))?,
        loc: get_loc(r)?,
        file: r.string()?,
    })
}

// --- IrFunction / IrParam --------------------------------------------------

fn encode_param(out: &mut Vec<u8>, p: &IrParam) {
    put_str(out, &p.name);
    put_str(out, &p.type_.name());
    put_opt_value(out, &p.default);
    put_loc(out, p.loc);
}

fn decode_param(r: &mut IrReader) -> Result<IrParam, String> {
    Ok(IrParam {
        name: r.string()?,
        type_: ParameterType::parse(&r.string()?),
        default: r.opt_value()?,
        loc: get_loc(r)?,
    })
}

fn encode_function(out: &mut Vec<u8>, f: &IrFunction) {
    put_str(out, &f.name);
    put_str(out, &f.visibility);
    put_str(out, &f.kind);
    put_bool(out, f.isolated);
    put_vec(out, &f.params, encode_param);
    put_str(out, &f.returns.name());
    put_vec(out, &f.body, encode_op);
    put_str(out, &f.file);
    put_loc(out, f.loc);
    // Resource ownership decisions (escape analysis, §15.6): a decoded package
    // has no AST to re-run escape analysis on, so this must ride the format for
    // the package-path RES/ownership rules and resource codegen to stay correct
    // (format v4). Sorted by binding name for a deterministic encoding.
    let mut owners: Vec<(&String, &crate::ir::resource_escape::ResOwner)> =
        f.resource_owners.iter().collect();
    owners.sort_by(|a, b| a.0.cmp(b.0));
    put_vec(out, &owners, |o, (name, owner)| {
        put_str(o, name);
        match owner {
            crate::ir::resource_escape::ResOwner::Local => put_u8(o, 0),
            crate::ir::resource_escape::ResOwner::Float(target) => {
                put_u8(o, 1);
                put_str(o, target);
            }
            // bug-291: unreachable on the wire -- `ir::verify` rejects a program
            // carrying a blocked float before it can be encoded. Encoded as
            // `Local` so the format stays v4-compatible rather than gaining a tag
            // no reader could ever legitimately see.
            crate::ir::resource_escape::ResOwner::FloatBlocked(_) => put_u8(o, 0),
        }
    });
}

fn decode_function(r: &mut IrReader) -> Result<IrFunction, String> {
    Ok(IrFunction {
        name: r.string()?,
        visibility: r.string()?,
        kind: r.string()?,
        isolated: r.bool()?,
        params: decode_vec(r, decode_param)?,
        returns: ParameterType::parse(&r.string()?),
        body: decode_vec(r, decode_op)?,
        file: r.string()?,
        loc: get_loc(r)?,
        resource_owners: decode_resource_owners(r)?,
    })
}

fn decode_resource_owners(
    r: &mut IrReader,
) -> Result<HashMap<String, crate::ir::resource_escape::ResOwner>, String> {
    let count = r.count()?;
    // Never pre-reserve more slots than the remaining bytes could fill (PKG-05,
    // mirroring `decode_vec`): each entry is a length-prefixed string + a 1-byte
    // owner tag, so it consumes at least one wire byte. Without this bound a
    // crafted `count` of `0xFFFF_FFFF` behind a tiny body makes
    // `HashMap::with_capacity` request ~100 GB — a clean `Err` on lazy-overcommit
    // platforms but a hard `SIGABRT` on Linux. The map still grows to the true
    // length; a bogus count now surfaces as the graceful truncation `Err` below.
    let mut owners = HashMap::with_capacity(count.min(r.bytes.len().saturating_sub(r.pos)));
    for _ in 0..count {
        let name = r.string()?;
        let owner = match r.u8()? {
            0 => crate::ir::resource_escape::ResOwner::Local,
            1 => crate::ir::resource_escape::ResOwner::Float(r.string()?),
            other => {
                return Err(format!(
                    "Binary Representation: invalid ResOwner tag {other}"
                ))
            }
        };
        owners.insert(name, owner);
    }
    Ok(owners)
}

// ===========================================================================
// Package IR merge
// ===========================================================================
//
// A consumer decodes each imported package's Binary Representation back to an `IrProject`
// and merges it into the project that flows through `IR -> NIR -> native`. To
// keep symbols unambiguous, a package's functions and globals are namespaced by
// the package name (`pkg.symbol`) — exactly how the consumer already names them
// (a `functionRef "thread_workers.echoText"` resolves to the merged function
// `thread_workers.echoText`). Imported *types* are referenced by their bare name
// by consumers, so type names stay unqualified and are de-duplicated by name.
// Every internal reference inside the package (sibling calls, function refs,
// global loads/stores) is rewritten to the namespaced form consistently.

/// Verify a freshly decoded package `IrProject` before it is merged into the
/// consuming project. The decoder already rejects a wrong magic/version
/// (`PACKAGE_BINARY_REPRESENTATION_VERSION_UNSUPPORTED`) and malformed bytes
/// (`PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED`); this pass re-states the package-format
/// invariants at the IR level (the structured form makes them direct checks
/// rather than CFG reconstruction). Checks here are conservative — they must
/// never reject IR this compiler legitimately produced — and surface as
/// `PACKAGE_BINARY_REPRESENTATION_VERIFY_*` diagnostics.
pub fn verify_package(pir: &IrProject) -> Result<(), String> {
    // Structural well-formedness: names are non-empty and functions are unique
    // (the link-time identity prefix relies on a function appearing once).
    let mut seen_functions: HashSet<&str> = HashSet::new();
    for function in &pir.functions {
        if function.name.is_empty() {
            return Err(format!(
                "{}: package contains an unnamed function",
                super::VERIFY_TYPE
            ));
        }
        if !seen_functions.insert(function.name.as_str()) {
            return Err(format!(
                "{}: duplicate function `{}` in package `{}`",
                super::VERIFY_TYPE,
                function.name,
                pir.name
            ));
        }
    }
    let mut seen_types: HashSet<&str> = HashSet::new();
    for ty in &pir.types {
        if !seen_types.insert(ty.name.as_str()) {
            return Err(format!(
                "{}: duplicate type `{}` in package `{}`",
                super::VERIFY_TYPE,
                ty.name,
                pir.name
            ));
        }
    }
    // Control-flow / trap structure: every MATCH must carry at least one case
    // (an empty MATCH cannot be exhaustive), and is checked recursively.
    for function in &pir.functions {
        verify_ops(&function.body, 0)?;
    }
    Ok(())
}

fn verify_ops(ops: &[IrOp], depth: usize) -> Result<(), String> {
    // Defensive depth cap mirroring the decoder (PKG-03): `verify_package` runs
    // on merged IR, which may not have flowed through the depth-bounded decoder.
    if depth > MAX_DECODE_DEPTH {
        return Err(format!(
            "{}: statement nesting exceeds the {MAX_DECODE_DEPTH} level limit",
            super::VERIFY_TYPE
        ));
    }
    for op in ops {
        match op {
            IrOp::If {
                then_body,
                else_body,
                ..
            } => {
                verify_ops(then_body, depth + 1)?;
                verify_ops(else_body, depth + 1)?;
            }
            IrOp::While { body, .. }
            | IrOp::For { body, .. }
            | IrOp::DoUntil { body, .. }
            | IrOp::ForEach { body, .. }
            | IrOp::Trap { body, .. } => verify_ops(body, depth + 1)?,
            IrOp::Match { cases, .. } => {
                // Structural pre-merge guard: reject a decoded package whose
                // MATCH carries no cases. `ir::verify` independently re-checks
                // this on the merged IR (its `VERIFY_MATCH` emit) — the two
                // enforcement points are intentional (pre-merge per-package vs.
                // the plan-20 post-merge sole-rejecter) and share one rule
                // id/message via the hoisted consts so they cannot drift.
                if cases.is_empty() {
                    return Err(format!(
                        "{}: {}",
                        super::VERIFY_MATCH,
                        super::VERIFY_MATCH_EMPTY_MSG
                    ));
                }
                for case in cases {
                    verify_ops(&case.body, depth + 1)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod value_op_tests {
    use super::*;

    /// Hand-assemble an `IrValue` node's bytes with an arbitrary operator
    /// spelling. `encode_value` can no longer produce one — `BinaryOp`/`UnaryOp`
    /// have no variant for it — which is exactly why the decoder still has to be
    /// tested against bytes a hostile or corrupt `.mfp` could carry.
    fn value_bytes_with_op(tag: u8, op: &str, unary: bool) -> Vec<u8> {
        let mut out = Vec::new();
        put_u8(&mut out, tag);
        put_str(&mut out, op);
        // One operand for a unary node, two for a binary one: `Const "1"` typed
        // `Integer` (tag 0), matching `encode_value`'s layout.
        let operands = if unary { 1 } else { 2 };
        for _ in 0..operands {
            put_u8(&mut out, 0);
            put_str(&mut out, "Integer");
            put_str(&mut out, "1");
        }
        put_str(&mut out, "Integer");
        put_u32(&mut out, 1);
        put_u32(&mut out, 1);
        out
    }

    /// plan-112, inheriting bug-403's guarantee for `IrValue`: an operator that
    /// arrives off the wire outside the vocabulary must be rejected at decode,
    /// never accepted and silently lowered as some other operator. In-memory the
    /// state is now unrepresentable; on the wire it is still a sequence of bytes,
    /// so this is the boundary where it has to be refused.
    #[test]
    fn decode_rejects_garbage_binary_and_unary_ops() {
        for (tag, unary, label) in [(18u8, false, "binary"), (19u8, true, "unary")] {
            for op in ["GARBAGE", "", "~", "and", "=="] {
                let bytes = value_bytes_with_op(tag, op, unary);
                let err = match decode_value(&mut IrReader::new(&bytes)) {
                    Ok(_) => {
                        panic!("{label} operator {op:?} must be rejected at decode, but it was accepted")
                    }
                    Err(err) => err,
                };
                assert!(
                    err.contains("operator"),
                    "unexpected error message for {label} {op:?}: {err}"
                );
            }
        }
    }

    /// The counterpart: every spelling the encoder can emit still decodes, so
    /// the rejection above is not simply refusing everything.
    #[test]
    fn decode_accepts_every_valid_operator() {
        for op in [
            BinaryOp::Or,
            BinaryOp::Xor,
            BinaryOp::And,
            BinaryOp::Equal,
            BinaryOp::NotEqual,
            BinaryOp::Less,
            BinaryOp::LessEqual,
            BinaryOp::Greater,
            BinaryOp::GreaterEqual,
            BinaryOp::Concat,
            BinaryOp::Add,
            BinaryOp::Subtract,
            BinaryOp::Multiply,
            BinaryOp::Divide,
            BinaryOp::Mod,
            BinaryOp::IntDiv,
            BinaryOp::Power,
        ] {
            let bytes = value_bytes_with_op(18, op.name(), false);
            match decode_value(&mut IrReader::new(&bytes)) {
                Ok(IrValue::Binary { op: got, .. }) => assert_eq!(got, op),
                Ok(_) => panic!("binary operator {op:?} decoded as some other node"),
                Err(err) => panic!("binary operator {op:?} did not decode: {err}"),
            }
        }
        for op in [UnaryOp::Not, UnaryOp::Negate, UnaryOp::SizeOf] {
            let bytes = value_bytes_with_op(19, op.name(), true);
            match decode_value(&mut IrReader::new(&bytes)) {
                Ok(IrValue::Unary { op: got, .. }) => assert_eq!(got, op),
                Ok(_) => panic!("unary operator {op:?} decoded as some other node"),
                Err(err) => panic!("unary operator {op:?} did not decode: {err}"),
            }
        }
    }
}

#[cfg(test)]
mod link_expr_op_tests {
    use super::link_codec::{decode_link_expr, encode_link_expr};
    use super::*;

    fn roundtrip(expr: &IrLinkExpr) -> Result<IrLinkExpr, String> {
        let mut bytes = Vec::new();
        encode_link_expr(&mut bytes, expr);
        decode_link_expr(&mut IrReader::new(&bytes))
    }

    // bug-403: a decoded `Compare` operator outside the six valid strings must be
    // rejected at decode — exactly like an unknown `AbiDirection` — never accepted
    // and silently treated as `=` at codegen.
    #[test]
    fn decode_rejects_garbage_compare_op() {
        let garbage = IrLinkExpr::Compare {
            op: "GARBAGE".to_string(),
            lhs: Box::new(IrLinkExpr::Int(0)),
            rhs: Box::new(IrLinkExpr::Int(0)),
        };
        let err = match roundtrip(&garbage) {
            Ok(_) => panic!("garbage Compare op must be rejected at decode, but it was accepted"),
            Err(err) => err,
        };
        assert!(
            err.contains("GARBAGE") || err.to_lowercase().contains("operator"),
            "unexpected error message: {err}"
        );
    }

    // The six valid operators still round-trip unchanged.
    #[test]
    fn decode_accepts_valid_compare_ops() {
        for op in ["=", "<>", "<", ">", "<=", ">="] {
            let expr = IrLinkExpr::Compare {
                op: op.to_string(),
                lhs: Box::new(IrLinkExpr::Var("status".to_string())),
                rhs: Box::new(IrLinkExpr::Int(0)),
            };
            let decoded = match roundtrip(&expr) {
                Ok(decoded) => decoded,
                Err(err) => panic!("valid Compare op `{op}` must decode, got error: {err}"),
            };
            match decoded {
                IrLinkExpr::Compare { op: got, .. } => assert_eq!(got, op),
                _ => panic!("expected Compare for op `{op}`"),
            }
        }
    }
}
