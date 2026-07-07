//! plan-01-vector: inline the pure-arithmetic `vector::` ops over the small
//! Float vectors as their equivalent constructor / arithmetic expression, so the
//! op lowers in place instead of through an out-of-line `#vector_<op>_<type>`
//! FUNC call.
//!
//! The rewrite reproduces the exact expression tree of the op's body in
//! `vector_package.mfb` (e.g. `scale` -> `Float3[a.x*b.x, a.y*b.y, a.z*b.z]`,
//! `dot` -> `a.x*b.x + a.y*b.y + a.z*b.z`), so the result and its
//! finiteness-observation (each lane / the final sum) are **bit-identical** to
//! the FUNC path — it is lowered through the same tested `lower_value` pipeline.
//! Only ops whose body is pure Float arithmetic are handled, and only when every
//! operand is cheap and side-effect-free to re-evaluate (the field reads
//! duplicate each operand once per lane); anything else falls back to the FUNC.

use super::*;

/// The `_floatN` type suffix, its constructor type name, and its field names.
fn float_vector_shape(target: &str) -> Option<(&'static str, &'static [&'static str])> {
    if target.ends_with("_float2") {
        Some(("Float2", &["x", "y"]))
    } else if target.ends_with("_float3") {
        Some(("Float3", &["x", "y", "z"]))
    } else if target.ends_with("_float4") {
        Some(("Float4", &["x", "y", "z", "w"]))
    } else {
        None
    }
}

/// Whether `value` is cheap and side-effect-free to evaluate more than once (a
/// binding read or a field read of one). A call/arithmetic operand is not — it
/// would be recomputed once per lane — so those fall back to the FUNC path.
fn is_reevaluation_safe(value: &NirValue) -> bool {
    match value {
        NirValue::Local(_) | NirValue::Global { .. } | NirValue::Const { .. } => true,
        NirValue::MemberAccess { target, .. } => is_reevaluation_safe(target),
        _ => false,
    }
}

impl CodeBuilder<'_> {
    /// Read `operand.<field>` as a synthetic `MemberAccess`.
    fn vector_field(operand: &NirValue, field: &str) -> NirValue {
        NirValue::MemberAccess {
            target: Box::new(operand.clone()),
            member: field.to_string(),
        }
    }

    /// Try to inline a `vector::` op call. Returns `Ok(Some(result))` when the op
    /// was inlined, `Ok(None)` to fall back to the ordinary FUNC-call lowering.
    pub(super) fn try_inline_vector_op(
        &mut self,
        target: &str,
        args: &[NirValue],
        loc: NirSourceLoc,
    ) -> Result<Option<ValueResult>, String> {
        let Some(op) = target.strip_prefix("#vector_") else {
            return Ok(None);
        };
        let Some((type_name, fields)) = float_vector_shape(target) else {
            return Ok(None);
        };
        // `op` still carries the `_floatN` suffix; keep only the op name.
        let op = op
            .strip_suffix("_float2")
            .or_else(|| op.strip_suffix("_float3"))
            .or_else(|| op.strip_suffix("_float4"))
            .unwrap_or(op);

        // Only the pure-Float-arithmetic ops are inlined; every operand must be
        // re-evaluation-safe (the field reads duplicate it per lane).
        if !args.iter().all(is_reevaluation_safe) {
            return Ok(None);
        }
        // A binary `op x` node over two synthetic operands at the call's location.
        let bin = |op: &str, left: NirValue, right: NirValue| NirValue::Binary {
            op: op.to_string(),
            left: Box::new(left),
            right: Box::new(right),
            loc,
        };
        // Build a vector-returning result by constructing `type_name` from `lanes`.
        let build = |this: &mut Self, lanes: Vec<NirValue>| -> Result<ValueResult, String> {
            this.lower_value(&NirValue::Constructor {
                type_: type_name.to_string(),
                args: lanes,
            })
        };

        let inlined = match (op, args.len()) {
            // scale: Float_N[ a.f*b.f ] — the componentwise (Hadamard) product.
            ("scale", 2) => {
                let (a, b) = (&args[0], &args[1]);
                let lanes = fields
                    .iter()
                    .map(|f| bin("*", Self::vector_field(a, f), Self::vector_field(b, f)))
                    .collect();
                build(self, lanes)?
            }
            // dot: a.f0*b.f0 + a.f1*b.f1 + ... (left-associative, matching the FUNC).
            ("dot", 2) => {
                let (a, b) = (&args[0], &args[1]);
                let product =
                    |f: &str| bin("*", Self::vector_field(a, f), Self::vector_field(b, f));
                let mut sum = product(fields[0]);
                for f in &fields[1..] {
                    sum = bin("+", sum, product(f));
                }
                self.lower_value(&sum)?
            }
            // lerp_unclamped: Float_N[ a.f + (b.f - a.f) * t ] — pure arithmetic
            // (the clamped `lerp` bounds `t` first, so it is not inlined here).
            ("lerp_unclamped", 3) => {
                let (a, b, t) = (&args[0], &args[1], &args[2]);
                let lanes = fields
                    .iter()
                    .map(|f| {
                        let delta =
                            bin("-", Self::vector_field(b, f), Self::vector_field(a, f));
                        bin("+", Self::vector_field(a, f), bin("*", delta, t.clone()))
                    })
                    .collect();
                build(self, lanes)?
            }
            // cross (3D, two args): the standard right-handed cross product. The 2D
            // (1-arg perpendicular) and 4D (3-arg) forms have different shapes and
            // are left to the FUNC.
            ("cross", 2) if type_name == "Float3" => {
                let (a, b) = (&args[0], &args[1]);
                let m = |v: &NirValue, f: &str| Self::vector_field(v, f);
                let lanes = vec![
                    bin("-", bin("*", m(a, "y"), m(b, "z")), bin("*", m(a, "z"), m(b, "y"))),
                    bin("-", bin("*", m(a, "z"), m(b, "x")), bin("*", m(a, "x"), m(b, "z"))),
                    bin("-", bin("*", m(a, "x"), m(b, "y")), bin("*", m(a, "y"), m(b, "x"))),
                ];
                build(self, lanes)?
            }
            _ => return Ok(None),
        };
        // The synthetic node above registered a statement-scope pending temp for a
        // fresh block result; the enclosing `lower_value(Call)` wrapper will
        // register the *same* block again. Claim the inner registration now so the
        // block is tracked exactly once (a double registration frees the owner's
        // block early — the caller's `claim_pending_temp` pops only one).
        self.claim_pending_temp(&inlined);
        Ok(Some(inlined))
    }
}
