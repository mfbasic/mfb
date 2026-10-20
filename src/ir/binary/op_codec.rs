//! plan-176-N: the IR wire codec's op_codec. The container, project and declaration codecs and
//! `verify_package` live in `ir/binary/mod.rs`.

use super::*;

// --- IrOp ------------------------------------------------------------------

pub(super) fn encode_op(out: &mut Vec<u8>, op: &IrOp) {
    match op {
        IrOp::Bind {
            mutable,
            name,
            type_,
            value,
            explicit_type,
            loc,
        } => {
            put_u8(out, 0);
            put_bool(out, *mutable);
            put_str(out, name);
            put_str(out, &type_.name());
            put_opt_value(out, value);
            put_bool(out, *explicit_type);
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
        IrOp::ExitLoop { kind, loc, target } => {
            put_u8(out, 11);
            put_loop_kind(out, *kind);
            put_opt_str(out, target);
            put_loc(out, *loc);
        }
        IrOp::ContinueLoop { kind, loc, target } => {
            put_u8(out, 12);
            put_loop_kind(out, *kind);
            put_opt_str(out, target);
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
            put_str(out, &type_.name());
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
            put_str(out, &type_.name());
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

pub(super) fn decode_op(r: &mut IrReader) -> Result<IrOp, String> {
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
            type_: ParameterType::parse(&r.string()?),
            value: r.opt_value()?,
            explicit_type: r.bool()?,
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
            target: r.opt_string()?,
            loc: get_loc(r)?,
        },
        12 => IrOp::ContinueLoop {
            kind: decode_loop_kind(r)?,
            target: r.opt_string()?,
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
            type_: ParameterType::parse(&r.string()?),
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
            type_: ParameterType::parse(&r.string()?),
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
