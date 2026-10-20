//! plan-176-O: `LINK` expressions (`emit_link_expr`) and their boolean form. The link support
//! blob, `ThunkContext` and `lower_link_initializer` live in
//! `codegen/link/thunk/link_thunk/mod.rs`.

use super::*;

/// Emit code computing the boolean/integer value of a `SUCCESS_ON`/`RESULT`
/// expression, reading the native return variable from `[sp + status_off]`.
///
/// Every intermediate is a fresh virtual register minted from the thunk's `vregs`
/// that the shared linear-scan allocator places and spills — the same discipline
/// as the rest of the thunk (plan-00-G). This replaces the historical scheme that
/// hand-assigned escalating *physical* registers (`x{base}`, `+2`, `+4`) with no
/// bound: a moderately right-nested tree walked `base` up into `x19`, the pinned
/// arena-base register, corrupting the arena program-wide (bug-56). Vregs never
/// touch reserved/callee-saved registers the thunk does not save. Returns the
/// name of the vreg holding the expression's value (0/1 for comparisons).
/// Emit a `SUCCESS_ON`/`RESULT` expression, resolving each `Var(name)` through
/// `offsets` to the frame slot holding that value (plan-50-I).
///
/// `offsets` maps every ABI slot name, and the ABI return name, to its frame
/// offset. An absent name is unreachable: both checkers reject a `Var` naming no
/// slot before codegen runs.
pub(super) fn emit_link_expr(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vregs: &mut Vregs,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> VirtualRegister {
    let dst = vregs.next();
    match expr {
        IrLinkExpr::Int(value) => {
            instructions.push(abi::move_immediate(
                &dst,
                "Integer",
                &(*value as u64).to_string(),
            ));
        }
        // plan-58-B: integer arithmetic, so a `BUFFER … SIZE` can scale a
        // frame/element count to bytes (`frames * channels * 2`) and a `LENGTH`
        // can scale a callee's element count back. Two's-complement wrapping,
        // like every other integer path here; the results are range-gated by
        // their consumers (SIZE against CBUFFER_MAX_BYTES, LENGTH clamped to
        // capacity), so an overflow cannot become an out-of-bounds size.
        IrLinkExpr::Mul(lhs, rhs) | IrLinkExpr::Add(lhs, rhs) | IrLinkExpr::Sub(lhs, rhs) => {
            let lhs_reg = emit_link_expr(lhs, offsets, vregs, symbol, counter, instructions);
            let rhs_reg = emit_link_expr(rhs, offsets, vregs, symbol, counter, instructions);
            instructions.push(match expr {
                IrLinkExpr::Mul(..) => abi::multiply_registers(&dst, &lhs_reg, &rhs_reg),
                IrLinkExpr::Add(..) => abi::add_registers(&dst, &lhs_reg, &rhs_reg),
                _ => abi::subtract_registers(&dst, &lhs_reg, &rhs_reg),
            });
        }
        IrLinkExpr::Var(name) => {
            let off = offsets.get(name.as_str()).copied().unwrap_or_else(|| {
                unreachable!(
                    "LINK expr names slot `{name}`, which verification should have rejected"
                )
            });
            instructions.push(abi::load_u64(&dst, abi::stack_pointer(), off));
        }
        IrLinkExpr::Not(inner) => {
            let inner_reg = emit_link_expr(inner, offsets, vregs, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let set = format!("{symbol}_not{id}_zero");
            let end = format!("{symbol}_not{id}_end");
            instructions.extend([
                abi::compare_immediate(&inner_reg, "0"),
                abi::branch_eq(&set),
                abi::move_immediate(&dst, "Integer", "0"),
                abi::branch(&end),
                abi::label(&set),
                abi::move_immediate(&dst, "Integer", "1"),
                abi::label(&end),
            ]);
        }
        IrLinkExpr::Compare { op, lhs, rhs } => {
            let lhs_reg = emit_link_expr(lhs, offsets, vregs, symbol, counter, instructions);
            let rhs_reg = emit_link_expr(rhs, offsets, vregs, symbol, counter, instructions);
            let id = *counter;
            *counter += 1;
            let end = format!("{symbol}_cmp{id}_end");
            // `IrLinkExpr::Compare` carries the operator as the wire spelling,
            // so this is a decode-side parse, not a stage re-deciding on a
            // string: the frontend's `lower_link_expr` only ever emits the six
            // comparisons, and the package (`.mfp`) decoder rejects any other
            // via `link_compare_op_valid` (bug-403). A loud panic here (never a
            // silent `=`) keeps a future decode gap from miscompiling.
            let branch = match BinaryOp::parse(op) {
                Some(BinaryOp::Equal) => abi::branch_eq(&end),
                Some(BinaryOp::NotEqual) => abi::branch_ne(&end),
                Some(BinaryOp::Less) => abi::branch_lt(&end),
                Some(BinaryOp::Greater) => abi::branch_gt(&end),
                Some(BinaryOp::LessEqual) => abi::branch_le(&end),
                Some(BinaryOp::GreaterEqual) => abi::branch_ge(&end),
                _ => unreachable!("invalid LINK Compare operator {op:?} reached codegen"),
            };
            instructions.push(abi::compare_registers(&lhs_reg, &rhs_reg));
            instructions.push(abi::move_immediate(&dst, "Integer", "1"));
            instructions.push(branch);
            instructions.push(abi::move_immediate(&dst, "Integer", "0"));
            instructions.push(abi::label(&end));
        }
        IrLinkExpr::And(lhs, rhs) => {
            // `AND`/`OR` are *logical* connectives: any nonzero operand is true. A
            // bare `Var`/`Int` leaf is an arbitrary integer, so combining with a raw
            // bitwise `and`/`or` would compute e.g. `2 & 1 = 0` and wrongly report
            // two truthy operands as false. Normalize each operand to a canonical
            // `0`/`1` first so bitwise coincides with logical (bug-66). Comparison /
            // logical sub-expressions already yield `0`/`1`, so they pass through
            // unchanged and `AND`/`OR`-of-comparisons stays byte-identical.
            let lhs_reg = emit_link_bool(lhs, offsets, vregs, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vregs, symbol, counter, instructions);
            instructions.push(abi::and_registers(&dst, &lhs_reg, &rhs_reg));
        }
        IrLinkExpr::Or(lhs, rhs) => {
            let lhs_reg = emit_link_bool(lhs, offsets, vregs, symbol, counter, instructions);
            let rhs_reg = emit_link_bool(rhs, offsets, vregs, symbol, counter, instructions);
            instructions.push(abi::or_registers(&dst, &lhs_reg, &rhs_reg));
        }
    }
    dst
}

/// True when `expr` already produces a canonical `0`/`1` truth value, so an
/// `AND`/`OR` operand of this shape needs no normalization. Only the bare `Var`
/// and `Int` leaves carry an arbitrary integer.
fn link_expr_is_boolean(expr: &IrLinkExpr) -> bool {
    matches!(
        expr,
        IrLinkExpr::Not(_)
            | IrLinkExpr::Compare { .. }
            | IrLinkExpr::And(_, _)
            | IrLinkExpr::Or(_, _)
    )
}

/// Emit `expr` and guarantee its value is a canonical `0`/`1` truth value for use
/// as an `AND`/`OR` operand. Sub-expressions that already yield `0`/`1`
/// (comparisons and boolean connectives) pass through unchanged; a bare
/// `Var`/`Int` leaf is normalized with a compare-nonzero so any nonzero value
/// becomes `1` (bug-66).
fn emit_link_bool(
    expr: &IrLinkExpr,
    offsets: &HashMap<&str, usize>,
    vregs: &mut Vregs,
    symbol: &str,
    counter: &mut usize,
    instructions: &mut Vec<CodeInstruction>,
) -> VirtualRegister {
    let value = emit_link_expr(expr, offsets, vregs, symbol, counter, instructions);
    if link_expr_is_boolean(expr) {
        return value;
    }
    let dst = vregs.next();
    let id = *counter;
    *counter += 1;
    let nonzero = format!("{symbol}_bool{id}_nz");
    let end = format!("{symbol}_bool{id}_end");
    instructions.extend([
        abi::compare_immediate(&value, "0"),
        abi::branch_ne(&nonzero),
        abi::move_immediate(&dst, "Integer", "0"),
        abi::branch(&end),
        abi::label(&nonzero),
        abi::move_immediate(&dst, "Integer", "1"),
        abi::label(&end),
    ]);
    dst
}
