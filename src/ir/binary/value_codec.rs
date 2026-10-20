//! plan-176-N: the IR wire codec's value_codec. The container, project and declaration codecs and
//! `verify_package` live in `ir/binary/mod.rs`.

use super::*;

// --- IrValue / IrRecordUpdate ----------------------------------------------

pub(super) fn encode_value(out: &mut Vec<u8>, v: &IrValue) {
    match v {
        IrValue::Const { type_, value } => {
            put_u8(out, 0);
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
        }
        IrValue::Closure {
            name,
            type_,
            captures,
        } => {
            put_u8(out, 4);
            put_str(out, name);
            put_str(out, &type_.name());
            put_vec(out, captures, encode_value);
        }
        IrValue::Capture {
            index,
            type_,
            by_ref,
        } => {
            put_u8(out, 5);
            put_u32(out, *index);
            put_str(out, &type_.name());
            put_bool(out, *by_ref);
        }
        IrValue::LocalRef { name, type_ } => {
            put_u8(out, 20);
            put_str(out, name);
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
            put_loc(out, *loc);
        }
        // bug-471: append-only tag, exactly as `SetLiteral` (21) was added — a
        // decoder that has never seen a `Checked` node simply rejects the tag,
        // and every already-committed `.mfp` re-encodes byte-identically.
        // bug-798: a capture-only `Checked` (no finiteness observation) is its
        // own append-only tag, so every observing one keeps tag 22 and every
        // already-committed `.mfp` still re-encodes byte-identically.
        IrValue::Checked {
            type_,
            value,
            observe,
        } => {
            put_u8(out, if *observe { 22 } else { 23 });
            put_str(out, &type_.name());
            encode_value(out, value);
        }
        IrValue::Constructor { type_, args } => {
            put_u8(out, 8);
            put_str(out, &type_.name());
            put_vec(out, args, encode_value);
        }
        IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            put_u8(out, 9);
            put_str(out, &union_type.name());
            put_str(out, &member_type.name());
            encode_value(out, value);
        }
        IrValue::UnionExtract { type_, value } => {
            put_u8(out, 10);
            put_str(out, &type_.name());
            encode_value(out, value);
        }
        IrValue::ResultIsOk { value } => {
            put_u8(out, 11);
            encode_value(out, value);
        }
        IrValue::ResultValue { type_, value } => {
            put_u8(out, 12);
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
            encode_value(out, target);
            put_vec(out, updates, |o, u| {
                put_str(o, &u.field);
                encode_value(o, &u.value);
            });
        }
        IrValue::ListLiteral { type_, values } => {
            put_u8(out, 15);
            put_str(out, &type_.name());
            put_vec(out, values, encode_value);
        }
        IrValue::SetLiteral { type_, values } => {
            put_u8(out, 21);
            put_str(out, &type_.name());
            put_vec(out, values, encode_value);
        }
        IrValue::MapLiteral { type_, entries } => {
            put_u8(out, 16);
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
        }
        IrValue::Binary {
            op,
            left,
            right,
            type_,
            loc,
        } => {
            put_u8(out, 18);
            put_str(out, op.name());
            encode_value(out, left);
            encode_value(out, right);
            put_str(out, &type_.name());
            put_loc(out, *loc);
        }
        IrValue::Unary {
            op,
            operand,
            type_,
            loc,
        } => {
            put_u8(out, 19);
            put_str(out, op.name());
            encode_value(out, operand);
            put_str(out, &type_.name());
            put_loc(out, *loc);
        }
    }
}

pub(super) fn put_loc(out: &mut Vec<u8>, loc: IrSourceLoc) {
    put_u32(out, loc.line);
    put_u32(out, loc.column);
}

pub(super) fn get_loc(r: &mut IrReader) -> Result<IrSourceLoc, String> {
    let line = r.u32()?;
    let column = r.u32()?;
    Ok(IrSourceLoc { line, column })
}

pub(super) fn decode_value(r: &mut IrReader) -> Result<IrValue, String> {
    r.enter()?;
    let result = decode_value_body(r);
    r.leave();
    result
}

fn decode_value_body(r: &mut IrReader) -> Result<IrValue, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrValue::Const {
            type_: crate::types::ParameterType::parse(&r.string()?),
            value: r.string()?,
        },
        1 => IrValue::Local(r.string()?),
        2 => IrValue::Global(r.string()?),
        3 => IrValue::FunctionRef {
            name: r.string()?,
            type_: crate::types::ParameterType::parse(&r.string()?),
        },
        4 => IrValue::Closure {
            name: r.string()?,
            type_: crate::types::ParameterType::parse(&r.string()?),
            captures: decode_vec(r, decode_value)?,
        },
        5 => IrValue::Capture {
            index: r.u32()?,
            type_: crate::types::ParameterType::parse(&r.string()?),
            by_ref: r.bool()?,
        },
        6 => IrValue::Call {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            type_: crate::types::ParameterType::parse(&r.string()?),
            loc: get_loc(r)?,
        },
        7 => IrValue::CallResult {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            type_: crate::types::ParameterType::parse(&r.string()?),
            loc: get_loc(r)?,
        },
        tag @ (22 | 23) => IrValue::Checked {
            type_: crate::types::ParameterType::parse(&r.string()?),
            value: Box::new(decode_value(r)?),
            observe: tag == 22,
        },
        8 => IrValue::Constructor {
            type_: crate::types::ParameterType::parse(&r.string()?),
            args: decode_vec(r, decode_value)?,
        },
        9 => IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse(&r.string()?),
            member_type: crate::types::ParameterType::parse(&r.string()?),
            value: Box::new(decode_value(r)?),
        },
        10 => IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse(&r.string()?),
            value: Box::new(decode_value(r)?),
        },
        11 => IrValue::ResultIsOk {
            value: Box::new(decode_value(r)?),
        },
        12 => IrValue::ResultValue {
            type_: crate::types::ParameterType::parse(&r.string()?),
            value: Box::new(decode_value(r)?),
        },
        13 => IrValue::ResultError {
            value: Box::new(decode_value(r)?),
        },
        14 => IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse(&r.string()?),
            target: Box::new(decode_value(r)?),
            updates: decode_vec(r, |r| {
                Ok(IrRecordUpdate {
                    field: r.string()?,
                    value: decode_value(r)?,
                })
            })?,
        },
        15 => IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse(&r.string()?),
            values: decode_vec(r, decode_value)?,
        },
        21 => IrValue::SetLiteral {
            type_: crate::types::ParameterType::parse(&r.string()?),
            values: decode_vec(r, decode_value)?,
        },
        16 => IrValue::MapLiteral {
            type_: crate::types::ParameterType::parse(&r.string()?),
            entries: decode_vec(r, |r| {
                let k = decode_value(r)?;
                let v = decode_value(r)?;
                Ok((k, v))
            })?,
        },
        17 => IrValue::MemberAccess {
            target: Box::new(decode_value(r)?),
            member: r.string()?,
            type_: crate::types::ParameterType::parse(&r.string()?),
        },
        18 => IrValue::Binary {
            // The operator is a length-prefixed string on the wire, so this is
            // the one place a spelling still has to be turned back into the
            // enum. Anything outside the set is rejected here rather than
            // silently mis-lowered later (bug-403's guarantee, made structural).
            op: {
                let text = r.string()?;
                BinaryOp::parse(&text).ok_or_else(|| {
                    format!("Binary Representation: unknown binary operator `{text}`")
                })?
            },
            left: Box::new(decode_value(r)?),
            right: Box::new(decode_value(r)?),
            type_: crate::types::ParameterType::parse(&r.string()?),
            loc: get_loc(r)?,
        },
        19 => IrValue::Unary {
            op: {
                let text = r.string()?;
                UnaryOp::parse(&text).ok_or_else(|| {
                    format!("Binary Representation: unknown unary operator `{text}`")
                })?
            },
            operand: Box::new(decode_value(r)?),
            type_: crate::types::ParameterType::parse(&r.string()?),
            loc: get_loc(r)?,
        },
        20 => IrValue::LocalRef {
            name: r.string()?,
            type_: crate::types::ParameterType::parse(&r.string()?),
        },
        other => {
            return Err(format!(
                "Binary Representation: unknown IrValue tag {other}"
            ))
        }
    })
}
