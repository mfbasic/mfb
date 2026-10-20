//! plan-176-N: the IR wire codec's link_codec. The container, project and declaration codecs and
//! `verify_package` live in `ir/binary/mod.rs`.

use super::*;

/// plan-53: an optional, append-only trailer carrying each stateful native LINK
/// function's `return_state_type` and `bind_state`, so a CONSUMER re-emitting an
/// imported thunk reproduces the STATE record and the `BIND STATE` marshalling.
///
/// Written ONLY when at least one link function is stateful, so a package whose
/// `LINK` block has no stateful native resource (every existing binding) is
/// byte-identical to before — no version bump, no golden churn. A decoder that
/// reaches end-of-input after the CSTRUCT table simply leaves every link function
/// stateless (`decode_link_state_trailer`).
pub(super) fn encode_link_state_trailer(out: &mut Vec<u8>, functions: &[IrLinkFunction]) {
    let stateful: Vec<&IrLinkFunction> = functions
        .iter()
        .filter(|f| f.return_state_type.is_some() || f.bind_state.is_some())
        .collect();
    if stateful.is_empty() {
        return;
    }
    put_vec(out, &stateful, |o, f| {
        put_str(o, &f.alias);
        put_str(o, &f.name);
        put_opt_str(
            o,
            &f.return_state_type.as_ref().map(|t| t.name().into_owned()),
        );
        put_opt_str(o, &f.bind_state);
    });
}

/// Read the STATE trailer (if present) and patch the matching link functions.
/// Absent — at end-of-input right after the CSTRUCT table — in every package
/// built before plan-53 and every one with no stateful native resource.
pub(super) fn decode_link_state_trailer(
    r: &mut IrReader,
    functions: &mut [IrLinkFunction],
) -> Result<(), String> {
    if r.at_end() {
        return Ok(());
    }
    let entries = decode_vec(r, |r| {
        Ok((r.string()?, r.string()?, r.opt_string()?, r.opt_string()?))
    })?;
    for (alias, name, return_state_type, bind_state) in entries {
        if let Some(f) = functions
            .iter_mut()
            .find(|f| f.alias == alias && f.name == name)
        {
            f.return_state_type = return_state_type.as_deref().map(ParameterType::parse);
            f.bind_state = bind_state;
        }
    }
    Ok(())
}

pub(super) fn encode_link_function(out: &mut Vec<u8>, f: &IrLinkFunction) {
    put_str(out, &f.alias);
    put_str(out, &f.name);
    put_str(out, &f.library);
    put_str(out, &f.symbol);
    // plan-111-B: the LINK types are `ParameterType` in memory now; the wire
    // still carries the spelling, so they render here and parse on decode.
    // Boundary #2 (plan-111-A §2 "The five boundaries").
    put_vec(out, &f.params, |o, (name, type_)| {
        put_str(o, name);
        put_str(o, &type_.name());
    });
    put_str(out, &f.return_type.name());
    put_bool(out, f.return_resource);
    // return_state_type / bind_state ride the optional trailer, not this record —
    // see encode_link_state_trailer.
    put_vec(out, &f.abi_slots, |o, slot| {
        put_str(o, &slot.name);
        put_str(o, &slot.ctype.name());
        put_u8(o, slot.direction.code());
    });
    put_str(out, &f.abi_return_name);
    put_str(out, &f.abi_return_ctype.name());
    // plan-58-C: the CBuffer surface, appended to the positional record. Rides the
    // 5->6 bump for the same reason BIND IN rode 4->5 — the record has no
    // per-field tags, so a field cannot be added without a version break.
    //
    // `max_buffer_bytes` is deliberately NOT here: the allocation ceiling is the
    // CONSUMING project's project.json `options.maxBuffer`, since thunks are emitted when
    // an executable links. A package carrying one would let a binding raise an
    // application's memory ceiling on its behalf.
    put_vec(out, &f.buffers, |o, b| {
        put_str(o, &b.slot);
        encode_link_expr(o, &b.size);
    });
    encode_opt_link_expr(out, &f.result_length);
    // plan-50-E: BIND IN, as field 15 of the positional record. Rides plan-50-C's
    // 4->5 bump — the record has no per-field tags, so it could not be added later
    // without a second break.
    put_vec(out, &f.bind_in, |o, b| {
        put_str(o, &b.slot);
        put_vec(o, &b.fields, |o, fl| {
            put_str(o, &fl.name);
            put_opt_str(o, &fl.param);
            match fl.literal {
                Some(v) => {
                    put_u8(o, 1);
                    put_str(o, &v.to_string());
                }
                None => put_u8(o, 0),
            }
        });
    });
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

pub(super) fn encode_link_expr(out: &mut Vec<u8>, expr: &IrLinkExpr) {
    match expr {
        IrLinkExpr::Var(name) => {
            put_u8(out, 0);
            put_str(out, name);
        }
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
        // plan-58-B. Tags 6-8 are new; an older reader rejects them as an
        // invalid tag rather than misreading one, since `decode_link_expr_body`
        // has always erred on an unknown tag.
        IrLinkExpr::Mul(lhs, rhs) => {
            put_u8(out, 6);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
        IrLinkExpr::Add(lhs, rhs) => {
            put_u8(out, 7);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
        IrLinkExpr::Sub(lhs, rhs) => {
            put_u8(out, 8);
            encode_link_expr(out, lhs);
            encode_link_expr(out, rhs);
        }
    }
}

/// Most `CSTRUCT` declarations one package may carry, and most fields one struct
/// may declare (plan-50-C §4.4).
///
/// `put_vec` writes an attacker-controlled `u32` count, so an unbounded count is
/// an allocation primitive — the same reasoning as the rest of the decode
/// hardening (PKG-01..07). Both caps are ~10x any plausible binding.
const MAX_CSTRUCTS: usize = 256;

const MAX_CSTRUCT_FIELDS: usize = 64;

/// Most `BUFFER … SIZE` clauses one LINK function may carry (plan-58-C §4.2).
///
/// A wrapper cannot have more `OUT CBuffer` slots than the target's external
/// integer argument registers — 6 on x86-64 SysV, 8 elsewhere — so 16 is already
/// unreachable by real code. It exists to bound allocation on a crafted file, not
/// to constrain anything a person would write.
const MAX_LINK_BUFFERS: usize = 16;

/// `decode_vec` with a cap on the element count.
///
/// `put_vec` writes an attacker-controlled `u32`, so an uncapped count is an
/// allocation primitive; `label` names the table in the diagnostic.
fn decode_vec_capped<T, F: Fn(&mut IrReader) -> Result<T, String>>(
    r: &mut IrReader,
    max: usize,
    label: &str,
    f: F,
) -> Result<Vec<T>, String> {
    let count = r.count()?;
    if count > max {
        return Err(format!(
            "PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED: {count} {label} exceeds the {max} limit"
        ));
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(f(r)?);
    }
    Ok(out)
}

/// Decode the `CSTRUCT` table. Carries field names and ctypes only; the layout is
/// recomputed by `ir::verify`, which is why a crafted package cannot dictate an
/// offset.
pub(super) fn decode_cstructs(r: &mut IrReader) -> Result<Vec<crate::ir::IrCStruct>, String> {
    let count = r.count()?;
    if count > MAX_CSTRUCTS {
        return Err(format!(
            "PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED: {count} CSTRUCT declarations exceeds the {MAX_CSTRUCTS} limit"
        ));
    }
    let mut structs = Vec::with_capacity(count);
    for _ in 0..count {
        let alias = r.string()?;
        let name = r.string()?;
        let maps_to = ParameterType::parse(&r.string()?);
        let field_count = r.count()?;
        if field_count > MAX_CSTRUCT_FIELDS {
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED: CSTRUCT `{name}` declares {field_count} fields, over the {MAX_CSTRUCT_FIELDS} limit"
            ));
        }
        let mut fields = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            fields.push(crate::ir::IrCStructField {
                name: r.string()?,
                // plan-113: `ParameterType::parse` is TOTAL, so the codec cannot
                // fail here. A spelling outside the 16 decodes to a `Named` and is
                // rejected by `check_cstruct` with the same NATIVE_ABI_UNKNOWN_CTYPE
                // text and location as before -- a decode error would have moved
                // that diagnostic.
                ctype: ParameterType::parse(&r.string()?),
            });
        }
        structs.push(crate::ir::IrCStruct {
            alias,
            name,
            maps_to,
            fields,
        });
    }
    Ok(structs)
}

pub(super) fn decode_link_function(r: &mut IrReader) -> Result<IrLinkFunction, String> {
    Ok(IrLinkFunction {
        alias: r.string()?,
        name: r.string()?,
        library: r.string()?,
        symbol: r.string()?,
        params: decode_vec(r, |r| Ok((r.string()?, ParameterType::parse(&r.string()?))))?,
        return_type: ParameterType::parse(&r.string()?),
        return_resource: r.bool()?,
        // return_state_type / bind_state are patched from the optional trailer
        // after the whole project decodes (decode_link_state_trailer).
        return_state_type: None,
        bind_state: None,
        // plan-58-A ships no wire format for BUFFER clauses; plan-58-C owns the
        // encode/decode and its `BINARY_REPR_VERSION` bump. Empty is SAFE only
        // while `link_thunk` refuses to lower a `CBuffer` slot at all — a decoded
        // package whose CBuffer slot reached the marshaler with no capacity would
        // hand a C function a zero-length buffer. plan-58-C must land the wire
        // format in the SAME change that removes that refusal.
        // Stays `None`: the BIND STATE resource-slot NAME is source-only surface
        // syntax that binds nothing at the package boundary, so it never rode the
        // wire. Its check ran when this package was built from source.
        bind_state_resource: None,
        abi_slots: decode_vec(r, |r| {
            let name = r.string()?;
            // A slot ctype may legitimately be a CSTRUCT nominal, so it parses
            // as a full `ParameterType` (plan-113).
            let ctype = ParameterType::parse(&r.string()?);
            let code = r.u8()?;
            // An unknown direction must be an error, never a silent default: it
            // decides whether the callee writes through this slot.
            let direction = crate::ir::AbiDirection::from_code(code)
                .ok_or_else(|| format!("invalid ABI slot direction {code}"))?;
            Ok(IrAbiSlot {
                name,
                ctype,
                direction,
            })
        })?,
        abi_return_name: r.string()?,
        abi_return_ctype: ParameterType::parse(&r.string()?),
        // plan-58-C. Struct-literal fields are evaluated in WRITTEN order, which
        // for this decoder IS the wire order — so these must sit exactly where
        // `encode_link_function` writes them (after `abi_return_ctype`, before
        // `bind_in`), not where they read most naturally next to their siblings.
        buffers: decode_vec_capped(r, MAX_LINK_BUFFERS, "LINK buffers", |r| {
            Ok(crate::ir::IrBuffer {
                slot: r.string()?,
                size: decode_link_expr(r)?,
            })
        })?,
        result_length: decode_opt_link_expr(r)?,
        bind_in: decode_vec(r, |r| {
            Ok(crate::ir::IrBindIn {
                slot: r.string()?,
                fields: decode_vec(r, |r| {
                    let name = r.string()?;
                    let param = r.opt_string()?;
                    let literal = if r.u8()? != 0 {
                        Some(
                            r.string()?
                                .parse::<i64>()
                                .map_err(|_| "invalid BIND IN literal".to_string())?,
                        )
                    } else {
                        None
                    };
                    Ok(crate::ir::IrBindInField {
                        name,
                        param,
                        literal,
                    })
                })?,
            })
        })?,
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

pub(super) fn decode_link_expr(r: &mut IrReader) -> Result<IrLinkExpr, String> {
    r.enter()?;
    let result = decode_link_expr_body(r);
    r.leave();
    result
}

fn decode_link_expr_body(r: &mut IrReader) -> Result<IrLinkExpr, String> {
    match r.u8()? {
        0 => Ok(IrLinkExpr::Var(r.string()?)),
        1 => Ok(IrLinkExpr::Int(
            r.string()?
                .parse::<i64>()
                .map_err(|_| "invalid LINK expr int".to_string())?,
        )),
        2 => {
            let op = r.string()?;
            // An unknown comparison operator must be an error, never a silent
            // default: at codegen `link_thunk` matches these six strings and a
            // stray one would fall through and be treated as `=` (bug-403). This
            // mirrors the sibling `AbiDirection` guard above — a decoded LINK-expr
            // field with a closed value set is validated here, not trusted.
            if !crate::ir::link_compare_op_valid(&op) {
                return Err(format!("invalid LINK Compare operator {op:?}"));
            }
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
        6 => Ok(IrLinkExpr::Mul(
            Box::new(decode_link_expr(r)?),
            Box::new(decode_link_expr(r)?),
        )),
        7 => Ok(IrLinkExpr::Add(
            Box::new(decode_link_expr(r)?),
            Box::new(decode_link_expr(r)?),
        )),
        8 => Ok(IrLinkExpr::Sub(
            Box::new(decode_link_expr(r)?),
            Box::new(decode_link_expr(r)?),
        )),
        other => Err(format!("invalid LINK expr tag {other}")),
    }
}
