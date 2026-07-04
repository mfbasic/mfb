use super::*;

/// Magic bytes prefixing a Binary Representation payload.
pub const BINARY_REPR_MAGIC: &[u8; 4] = b"MFBR";
/// Binary Representation format version. Bump on any incompatible change to the encoding.
/// Version 2 adds per-node source locations (`loc` on Call/CallResult/Binary/Unary/For)
/// and a per-function source `file`, backing read-only `Error.source` / `ErrorLoc`.
/// Version 3 (plan-20-A) extends spans to the full diagnostic vocabulary: every
/// op, match case, and declaration (function/param/type/field/variant/binding)
/// carries a trailing `loc`, so the IR-level semantic checker can report at the
/// same source line the AST checker does.
pub const BINARY_REPR_VERSION: u16 = 3;

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
const MAX_DECODE_DEPTH: usize = 256;

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
            .map_or(true, |end| end > self.bytes.len())
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
            put_str(out, &entry.returns);
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
    if !project.link_functions.is_empty() || !project.link_aliases.is_empty() {
        put_vec(out, &project.link_functions, encode_link_function);
        put_vec(out, &project.link_aliases, |o, (alias, target)| {
            put_str(o, alias);
            put_str(o, target);
        });
    }
}

fn encode_link_function(out: &mut Vec<u8>, f: &IrLinkFunction) {
    put_str(out, &f.alias);
    put_str(out, &f.name);
    put_str(out, &f.library);
    put_str(out, &f.symbol);
    put_vec(out, &f.params, |o, (name, type_)| {
        put_str(o, name);
        put_str(o, type_);
    });
    put_str(out, &f.return_type);
    put_bool(out, f.return_resource);
    put_vec(out, &f.abi_slots, |o, slot| {
        put_str(o, &slot.name);
        put_str(o, &slot.ctype);
        put_bool(o, slot.is_out);
    });
    put_str(out, &f.abi_return_name);
    put_str(out, &f.abi_return_ctype);
    put_vec(out, &f.consts, |o, (slot, value)| {
        put_str(o, slot);
        put_str(o, &value.to_string());
    });
    encode_opt_link_expr(out, &f.success_on);
    encode_opt_link_expr(out, &f.result);
    match &f.free {
        Some(free) => {
            put_u8(out, 1);
            put_str(out, &free.slot);
            put_str(out, &free.symbol);
        }
        None => put_u8(out, 0),
    }
}

fn encode_opt_link_expr(out: &mut Vec<u8>, expr: &Option<IrLinkExpr>) {
    match expr {
        Some(expr) => {
            put_u8(out, 1);
            encode_link_expr(out, expr);
        }
        None => put_u8(out, 0),
    }
}

fn encode_link_expr(out: &mut Vec<u8>, expr: &IrLinkExpr) {
    match expr {
        IrLinkExpr::Var => put_u8(out, 0),
        IrLinkExpr::Int(value) => {
            put_u8(out, 1);
            put_str(out, &value.to_string());
        }
        IrLinkExpr::Compare { op, lhs, rhs } => {
            put_u8(out, 2);
            put_str(out, op);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
        IrLinkExpr::And(lhs, rhs) => {
            put_u8(out, 3);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
        IrLinkExpr::Or(lhs, rhs) => {
            put_u8(out, 4);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
        IrLinkExpr::Not(inner) => {
            put_u8(out, 5);
            encode_link_expr(out, inner);
        }
    }
}

fn decode_project(r: &mut IrReader) -> Result<IrProject, String> {
    let name = r.string()?;
    let entry = if r.u8()? != 0 {
        Some(EntryPoint {
            name: r.string()?,
            returns: r.string()?,
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
    let (link_functions, link_aliases) = if r.at_end() {
        (Vec::new(), Vec::new())
    } else {
        let link_functions = decode_vec(r, decode_link_function)?;
        let link_aliases = decode_vec(r, |r| Ok((r.string()?, r.string()?)))?;
        (link_functions, link_aliases)
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
        link_aliases,
        // Docs live in a separate optional package section, not in the decoded IR.
        docs: ProjectDocs::default(),
    })
}

fn decode_link_function(r: &mut IrReader) -> Result<IrLinkFunction, String> {
    Ok(IrLinkFunction {
        alias: r.string()?,
        name: r.string()?,
        library: r.string()?,
        symbol: r.string()?,
        params: decode_vec(r, |r| Ok((r.string()?, r.string()?)))?,
        return_type: r.string()?,
        return_resource: r.bool()?,
        abi_slots: decode_vec(r, |r| {
            Ok(IrAbiSlot {
                name: r.string()?,
                ctype: r.string()?,
                is_out: r.bool()?,
            })
        })?,
        abi_return_name: r.string()?,
        abi_return_ctype: r.string()?,
        consts: decode_vec(r, |r| {
            let slot = r.string()?;
            let value = r
                .string()?
                .parse::<i64>()
                .map_err(|_| "invalid LINK const value".to_string())?;
            Ok((slot, value))
        })?,
        success_on: decode_opt_link_expr(r)?,
        result: decode_opt_link_expr(r)?,
        free: if r.u8()? != 0 {
            Some(IrFree {
                slot: r.string()?,
                symbol: r.string()?,
            })
        } else {
            None
        },
    })
}

fn decode_opt_link_expr(r: &mut IrReader) -> Result<Option<IrLinkExpr>, String> {
    if r.u8()? != 0 {
        Ok(Some(decode_link_expr(r)?))
    } else {
        Ok(None)
    }
}

fn decode_link_expr(r: &mut IrReader) -> Result<IrLinkExpr, String> {
    r.enter()?;
    let result = decode_link_expr_body(r);
    r.leave();
    result
}

fn decode_link_expr_body(r: &mut IrReader) -> Result<IrLinkExpr, String> {
    match r.u8()? {
        0 => Ok(IrLinkExpr::Var),
        1 => Ok(IrLinkExpr::Int(
            r.string()?
                .parse::<i64>()
                .map_err(|_| "invalid LINK expr int".to_string())?,
        )),
        2 => {
            let op = r.string()?;
            let lhs = Box::new(decode_link_expr(r)?);
            let rhs = Box::new(decode_link_expr(r)?);
            Ok(IrLinkExpr::Compare { op, lhs, rhs })
        }
        3 => Ok(IrLinkExpr::And(
            Box::new(decode_link_expr(r)?),
            Box::new(decode_link_expr(r)?),
        )),
        4 => Ok(IrLinkExpr::Or(
            Box::new(decode_link_expr(r)?),
            Box::new(decode_link_expr(r)?),
        )),
        5 => Ok(IrLinkExpr::Not(Box::new(decode_link_expr(r)?))),
        other => Err(format!("invalid LINK expr tag {other}")),
    }
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
    put_str(out, &b.type_);
    put_opt_value(out, &b.value);
    put_loc(out, b.loc);
}

fn decode_binding(r: &mut IrReader) -> Result<IrBinding, String> {
    Ok(IrBinding {
        name: r.string()?,
        visibility: r.string()?,
        mutable: r.bool()?,
        type_: r.string()?,
        value: r.opt_value()?,
        loc: get_loc(r)?,
    })
}

// --- IrType / IrField / IrVariant / IrEnumMember ---------------------------

fn encode_field(out: &mut Vec<u8>, f: &IrField) {
    put_opt_str(out, &f.visibility);
    put_str(out, &f.name);
    put_str(out, &f.type_);
    put_loc(out, f.loc);
}

fn decode_field(r: &mut IrReader) -> Result<IrField, String> {
    Ok(IrField {
        visibility: r.opt_string()?,
        name: r.string()?,
        type_: r.string()?,
        loc: get_loc(r)?,
    })
}

fn encode_variant(out: &mut Vec<u8>, v: &IrVariant) {
    put_str(out, &v.name);
    put_vec(out, &v.fields, encode_field);
    put_loc(out, v.loc);
}

fn decode_variant(r: &mut IrReader) -> Result<IrVariant, String> {
    Ok(IrVariant {
        name: r.string()?,
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
    put_str(out, &p.type_);
    put_opt_value(out, &p.default);
    put_loc(out, p.loc);
}

fn decode_param(r: &mut IrReader) -> Result<IrParam, String> {
    Ok(IrParam {
        name: r.string()?,
        type_: r.string()?,
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
    put_str(out, &f.returns);
    put_vec(out, &f.body, encode_op);
    put_str(out, &f.file);
    put_loc(out, f.loc);
}

fn decode_function(r: &mut IrReader) -> Result<IrFunction, String> {
    Ok(IrFunction {
        name: r.string()?,
        visibility: r.string()?,
        kind: r.string()?,
        isolated: r.bool()?,
        params: decode_vec(r, decode_param)?,
        returns: r.string()?,
        body: decode_vec(r, decode_op)?,
        file: r.string()?,
        loc: get_loc(r)?,
        // Recomputed by codegen from the in-memory IR; not part of the binary
        // representation, so a decoded function carries an empty map.
        resource_owners: HashMap::new(),
    })
}

// --- IrOp ------------------------------------------------------------------

fn encode_op(out: &mut Vec<u8>, op: &IrOp) {
    match op {
        IrOp::Bind {
            mutable,
            name,
            type_,
            value,
            loc,
        } => {
            put_u8(out, 0);
            put_bool(out, *mutable);
            put_str(out, name);
            put_str(out, type_);
            put_opt_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::Assign { name, value, loc } => {
            put_u8(out, 1);
            put_str(out, name);
            encode_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::AssignGlobal { name, value, loc } => {
            put_u8(out, 2);
            put_str(out, name);
            encode_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::StateAssign {
            resource,
            value,
            loc,
        } => {
            put_u8(out, 17);
            put_str(out, resource);
            encode_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::Return { value, loc } => {
            put_u8(out, 3);
            put_opt_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::ExitLoop { kind, loc } => {
            put_u8(out, 11);
            put_loop_kind(out, *kind);
            put_loc(out, *loc);
        }
        IrOp::ContinueLoop { kind, loc } => {
            put_u8(out, 12);
            put_loop_kind(out, *kind);
            put_loc(out, *loc);
        }
        IrOp::ExitProgram { code, loc } => {
            put_u8(out, 13);
            encode_value(out, code);
            put_loc(out, *loc);
        }
        IrOp::Fail { error, loc } => {
            put_u8(out, 4);
            encode_value(out, error);
            put_loc(out, *loc);
        }
        IrOp::Eval { value, loc } => {
            put_u8(out, 5);
            encode_value(out, value);
            put_loc(out, *loc);
        }
        IrOp::If {
            condition,
            then_body,
            else_body,
            loc,
        } => {
            put_u8(out, 6);
            encode_value(out, condition);
            put_vec(out, then_body, encode_op);
            put_vec(out, else_body, encode_op);
            put_loc(out, *loc);
        }
        IrOp::Match { value, cases, loc } => {
            put_u8(out, 7);
            encode_value(out, value);
            put_vec(out, cases, encode_match_case);
            put_loc(out, *loc);
        }
        IrOp::While {
            kind,
            condition,
            body,
            loc,
        } => {
            if matches!(kind, LoopKind::While) {
                put_u8(out, 8);
            } else {
                put_u8(out, 16);
                put_loop_kind(out, *kind);
            }
            encode_value(out, condition);
            put_vec(out, body, encode_op);
            put_loc(out, *loc);
        }
        IrOp::For {
            name,
            type_,
            start,
            end,
            step,
            body,
            loc,
        } => {
            put_u8(out, 14);
            put_str(out, name);
            put_str(out, type_);
            encode_value(out, start);
            encode_value(out, end);
            encode_value(out, step);
            put_vec(out, body, encode_op);
            put_loc(out, *loc);
        }
        IrOp::DoUntil {
            body,
            condition,
            loc,
        } => {
            put_u8(out, 15);
            put_vec(out, body, encode_op);
            encode_value(out, condition);
            put_loc(out, *loc);
        }
        IrOp::ForEach {
            name,
            type_,
            iterable,
            body,
            loc,
        } => {
            put_u8(out, 9);
            put_str(out, name);
            put_str(out, type_);
            encode_value(out, iterable);
            put_vec(out, body, encode_op);
            put_loc(out, *loc);
        }
        IrOp::Trap { name, body, loc } => {
            put_u8(out, 10);
            put_str(out, name);
            put_vec(out, body, encode_op);
            put_loc(out, *loc);
        }
    }
}

fn decode_op(r: &mut IrReader) -> Result<IrOp, String> {
    r.enter()?;
    let result = decode_op_body(r);
    r.leave();
    result
}

fn decode_op_body(r: &mut IrReader) -> Result<IrOp, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrOp::Bind {
            mutable: r.bool()?,
            name: r.string()?,
            type_: r.string()?,
            value: r.opt_value()?,
            loc: get_loc(r)?,
        },
        1 => IrOp::Assign {
            name: r.string()?,
            value: decode_value(r)?,
            loc: get_loc(r)?,
        },
        2 => IrOp::AssignGlobal {
            name: r.string()?,
            value: decode_value(r)?,
            loc: get_loc(r)?,
        },
        17 => IrOp::StateAssign {
            resource: r.string()?,
            value: decode_value(r)?,
            loc: get_loc(r)?,
        },
        3 => IrOp::Return {
            value: r.opt_value()?,
            loc: get_loc(r)?,
        },
        11 => IrOp::ExitLoop {
            kind: decode_loop_kind(r)?,
            loc: get_loc(r)?,
        },
        12 => IrOp::ContinueLoop {
            kind: decode_loop_kind(r)?,
            loc: get_loc(r)?,
        },
        13 => IrOp::ExitProgram {
            code: decode_value(r)?,
            loc: get_loc(r)?,
        },
        4 => IrOp::Fail {
            error: decode_value(r)?,
            loc: get_loc(r)?,
        },
        5 => IrOp::Eval {
            value: decode_value(r)?,
            loc: get_loc(r)?,
        },
        6 => IrOp::If {
            condition: decode_value(r)?,
            then_body: decode_vec(r, decode_op)?,
            else_body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        7 => IrOp::Match {
            value: decode_value(r)?,
            cases: decode_vec(r, decode_match_case)?,
            loc: get_loc(r)?,
        },
        8 => IrOp::While {
            kind: LoopKind::While,
            condition: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        9 => IrOp::ForEach {
            name: r.string()?,
            type_: r.string()?,
            iterable: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        10 => IrOp::Trap {
            name: r.string()?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        14 => IrOp::For {
            name: r.string()?,
            type_: r.string()?,
            start: decode_value(r)?,
            end: decode_value(r)?,
            step: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        15 => IrOp::DoUntil {
            body: decode_vec(r, decode_op)?,
            condition: decode_value(r)?,
            loc: get_loc(r)?,
        },
        16 => IrOp::While {
            kind: decode_loop_kind(r)?,
            condition: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        other => return Err(format!("Binary Representation: unknown IrOp tag {other}")),
    })
}

fn decode_loop_kind(r: &mut IrReader) -> Result<LoopKind, String> {
    match r.u8()? {
        0 => Ok(LoopKind::For),
        1 => Ok(LoopKind::Do),
        2 => Ok(LoopKind::While),
        other => Err(format!(
            "Binary Representation: unknown loop kind tag {other}"
        )),
    }
}

// --- IrMatchCase / IrMatchPattern ------------------------------------------

fn encode_match_case(out: &mut Vec<u8>, c: &IrMatchCase) {
    encode_match_pattern(out, &c.pattern);
    put_opt_value(out, &c.guard);
    put_vec(out, &c.body, encode_op);
    put_loc(out, c.loc);
}

fn decode_match_case(r: &mut IrReader) -> Result<IrMatchCase, String> {
    Ok(IrMatchCase {
        pattern: decode_match_pattern(r)?,
        guard: r.opt_value()?,
        body: decode_vec(r, decode_op)?,
        loc: get_loc(r)?,
    })
}

fn encode_match_pattern(out: &mut Vec<u8>, p: &IrMatchPattern) {
    match p {
        IrMatchPattern::Else => put_u8(out, 0),
        IrMatchPattern::Value(v) => {
            put_u8(out, 1);
            encode_value(out, v);
        }
        IrMatchPattern::OneOf(vs) => {
            put_u8(out, 2);
            put_vec(out, vs, encode_value);
        }
    }
}

fn decode_match_pattern(r: &mut IrReader) -> Result<IrMatchPattern, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrMatchPattern::Else,
        1 => IrMatchPattern::Value(decode_value(r)?),
        2 => IrMatchPattern::OneOf(decode_vec(r, decode_value)?),
        other => {
            return Err(format!(
                "Binary Representation: unknown IrMatchPattern tag {other}"
            ))
        }
    })
}

// --- IrValue / IrRecordUpdate ----------------------------------------------

fn encode_value(out: &mut Vec<u8>, v: &IrValue) {
    match v {
        IrValue::Const { type_, value } => {
            put_u8(out, 0);
            put_str(out, type_);
            put_str(out, value);
        }
        IrValue::Local(name) => {
            put_u8(out, 1);
            put_str(out, name);
        }
        IrValue::Global(name) => {
            put_u8(out, 2);
            put_str(out, name);
        }
        IrValue::FunctionRef { name, type_ } => {
            put_u8(out, 3);
            put_str(out, name);
            put_str(out, type_);
        }
        IrValue::Closure {
            name,
            type_,
            captures,
        } => {
            put_u8(out, 4);
            put_str(out, name);
            put_str(out, type_);
            put_vec(out, captures, encode_value);
        }
        IrValue::Capture {
            index,
            type_,
            by_ref,
        } => {
            put_u8(out, 5);
            put_u32(out, *index as u32);
            put_str(out, type_);
            put_bool(out, *by_ref);
        }
        IrValue::LocalRef { name, type_ } => {
            put_u8(out, 20);
            put_str(out, name);
            put_str(out, type_);
        }
        IrValue::Call {
            target,
            args,
            type_,
            loc,
        } => {
            put_u8(out, 6);
            put_str(out, target);
            put_vec(out, args, encode_value);
            put_str(out, type_);
            put_loc(out, *loc);
        }
        IrValue::CallResult {
            target,
            args,
            type_,
            loc,
        } => {
            put_u8(out, 7);
            put_str(out, target);
            put_vec(out, args, encode_value);
            put_str(out, type_);
            put_loc(out, *loc);
        }
        IrValue::Constructor { type_, args } => {
            put_u8(out, 8);
            put_str(out, type_);
            put_vec(out, args, encode_value);
        }
        IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            put_u8(out, 9);
            put_str(out, union_type);
            put_str(out, member_type);
            encode_value(out, value);
        }
        IrValue::UnionExtract { type_, value } => {
            put_u8(out, 10);
            put_str(out, type_);
            encode_value(out, value);
        }
        IrValue::ResultIsOk { value } => {
            put_u8(out, 11);
            encode_value(out, value);
        }
        IrValue::ResultValue { type_, value } => {
            put_u8(out, 12);
            put_str(out, type_);
            encode_value(out, value);
        }
        IrValue::ResultError { value } => {
            put_u8(out, 13);
            encode_value(out, value);
        }
        IrValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            put_u8(out, 14);
            put_str(out, type_);
            encode_value(out, target);
            put_vec(out, updates, |o, u| {
                put_str(o, &u.field);
                encode_value(o, &u.value);
            });
        }
        IrValue::ListLiteral { type_, values } => {
            put_u8(out, 15);
            put_str(out, type_);
            put_vec(out, values, encode_value);
        }
        IrValue::MapLiteral { type_, entries } => {
            put_u8(out, 16);
            put_str(out, type_);
            put_vec(out, entries, |o, (k, val)| {
                encode_value(o, k);
                encode_value(o, val);
            });
        }
        IrValue::MemberAccess {
            target,
            member,
            type_,
        } => {
            put_u8(out, 17);
            encode_value(out, target);
            put_str(out, member);
            put_str(out, type_);
        }
        IrValue::Binary {
            op,
            left,
            right,
            type_,
            loc,
        } => {
            put_u8(out, 18);
            put_str(out, op);
            encode_value(out, left);
            encode_value(out, right);
            put_str(out, type_);
            put_loc(out, *loc);
        }
        IrValue::Unary {
            op,
            operand,
            type_,
            loc,
        } => {
            put_u8(out, 19);
            put_str(out, op);
            encode_value(out, operand);
            put_str(out, type_);
            put_loc(out, *loc);
        }
    }
}

fn put_loc(out: &mut Vec<u8>, loc: IrSourceLoc) {
    put_u32(out, loc.line);
    put_u32(out, loc.column);
}

fn get_loc(r: &mut IrReader) -> Result<IrSourceLoc, String> {
    let line = r.u32()?;
    let column = r.u32()?;
    Ok(IrSourceLoc { line, column })
}

fn decode_value(r: &mut IrReader) -> Result<IrValue, String> {
    r.enter()?;
    let result = decode_value_body(r);
    r.leave();
    result
}

fn decode_value_body(r: &mut IrReader) -> Result<IrValue, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrValue::Const {
            type_: r.string()?,
            value: r.string()?,
        },
        1 => IrValue::Local(r.string()?),
        2 => IrValue::Global(r.string()?),
        3 => IrValue::FunctionRef {
            name: r.string()?,
            type_: r.string()?,
        },
        4 => IrValue::Closure {
            name: r.string()?,
            type_: r.string()?,
            captures: decode_vec(r, decode_value)?,
        },
        5 => IrValue::Capture {
            index: r.u32()? as usize,
            type_: r.string()?,
            by_ref: r.bool()?,
        },
        6 => IrValue::Call {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            type_: r.string()?,
            loc: get_loc(r)?,
        },
        7 => IrValue::CallResult {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            type_: r.string()?,
            loc: get_loc(r)?,
        },
        8 => IrValue::Constructor {
            type_: r.string()?,
            args: decode_vec(r, decode_value)?,
        },
        9 => IrValue::UnionWrap {
            union_type: r.string()?,
            member_type: r.string()?,
            value: Box::new(decode_value(r)?),
        },
        10 => IrValue::UnionExtract {
            type_: r.string()?,
            value: Box::new(decode_value(r)?),
        },
        11 => IrValue::ResultIsOk {
            value: Box::new(decode_value(r)?),
        },
        12 => IrValue::ResultValue {
            type_: r.string()?,
            value: Box::new(decode_value(r)?),
        },
        13 => IrValue::ResultError {
            value: Box::new(decode_value(r)?),
        },
        14 => IrValue::WithUpdate {
            type_: r.string()?,
            target: Box::new(decode_value(r)?),
            updates: decode_vec(r, |r| {
                Ok(IrRecordUpdate {
                    field: r.string()?,
                    value: decode_value(r)?,
                })
            })?,
        },
        15 => IrValue::ListLiteral {
            type_: r.string()?,
            values: decode_vec(r, decode_value)?,
        },
        16 => IrValue::MapLiteral {
            type_: r.string()?,
            entries: decode_vec(r, |r| {
                let k = decode_value(r)?;
                let v = decode_value(r)?;
                Ok((k, v))
            })?,
        },
        17 => IrValue::MemberAccess {
            target: Box::new(decode_value(r)?),
            member: r.string()?,
            type_: r.string()?,
        },
        18 => IrValue::Binary {
            op: r.string()?,
            left: Box::new(decode_value(r)?),
            right: Box::new(decode_value(r)?),
            type_: r.string()?,
            loc: get_loc(r)?,
        },
        19 => IrValue::Unary {
            op: r.string()?,
            operand: Box::new(decode_value(r)?),
            type_: r.string()?,
            loc: get_loc(r)?,
        },
        20 => IrValue::LocalRef {
            name: r.string()?,
            type_: r.string()?,
        },
        other => {
            return Err(format!(
                "Binary Representation: unknown IrValue tag {other}"
            ))
        }
    })
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
            return Err(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: package contains an unnamed function"
                    .to_string(),
            );
        }
        if !seen_functions.insert(function.name.as_str()) {
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: duplicate function `{}` in package `{}`",
                function.name, pir.name
            ));
        }
    }
    let mut seen_types: HashSet<&str> = HashSet::new();
    for ty in &pir.types {
        if !seen_types.insert(ty.name.as_str()) {
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: duplicate type `{}` in package `{}`",
                ty.name, pir.name
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
            "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: statement nesting exceeds the {MAX_DECODE_DEPTH} level limit"
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
            IrOp::While { body, .. } | IrOp::ForEach { body, .. } | IrOp::Trap { body, .. } => {
                verify_ops(body, depth + 1)?
            }
            IrOp::Match { cases, .. } => {
                if cases.is_empty() {
                    return Err(
                        "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH: MATCH has no cases (not exhaustive)".to_string(),
                    );
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
