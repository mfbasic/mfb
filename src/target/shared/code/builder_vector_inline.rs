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
//! The pure-arithmetic ops (`scale`/`dot`/`cross`) are handled for every element
//! type — Float, Fixed, and Integer — since their re-lowered `*`/`+`/`-` trees are
//! bit-identical to the FUNC body (plan-39 C1). `lerp`/`length`/`distance` stay
//! Float-only (they use `math::sqrt` / Float clamp constants; the Fixed/Integer
//! bodies differ). Inlining fires only when every operand is cheap and
//! side-effect-free to re-evaluate (the field reads duplicate each operand once
//! per lane); anything else falls back to the FUNC.

use super::*;

/// The un-encodable prefix marking a `ValueResult.location` as a register-native
/// vector whose lanes live in the `vector_natives` side-table. Chosen so it can
/// never be a physical register / vreg / stack slot: if one leaks to a GP or
/// store site it hard-errors at the encoder (fail-loud) instead of miscompiling.
pub(super) const VECTOR_NATIVE_MARKER: &str = "%%vecnative:";

/// The lane count of a register-native small vector type (Float/Fixed/Integer),
/// or `None`. The carrier (construction, member reads, boundary materialization)
/// is element-type-agnostic — a lane is a scalar `Float`/`Fixed`/`Integer` value
/// stored as 8 bytes — so every `<Elem>N` type is register-native. Only the *op
/// inlining* is Float-only (Fixed/Integer ops keep their FUNC bodies).
pub(super) fn vector_field_count(type_name: &str) -> Option<usize> {
    match type_name {
        "Float2" | "Fixed2" | "Integer2" => Some(2),
        "Float3" | "Fixed3" | "Integer3" => Some(3),
        "Float4" | "Fixed4" | "Integer4" => Some(4),
        _ => None,
    }
}

/// Map a vector field name to its lane index.
fn vector_field_index(member: &str) -> Option<usize> {
    match member {
        "x" => Some(0),
        "y" => Some(1),
        "z" => Some(2),
        "w" => Some(3),
        _ => None,
    }
}

/// The nine register-native vector shapes: the `_<element><dim>` target suffix,
/// its constructor type name, its field names, and its element type. plan-39 C1
/// extends inlining from the Float shapes to the Fixed/Integer shapes for the
/// pure-arithmetic ops (see `vector_op_inlinable`).
const VECTOR_SHAPES: &[(&str, &str, &[&str], &str)] = &[
    ("_float2", "Float2", &["x", "y"], "Float"),
    ("_float3", "Float3", &["x", "y", "z"], "Float"),
    ("_float4", "Float4", &["x", "y", "z", "w"], "Float"),
    ("_fixed2", "Fixed2", &["x", "y"], "Fixed"),
    ("_fixed3", "Fixed3", &["x", "y", "z"], "Fixed"),
    ("_fixed4", "Fixed4", &["x", "y", "z", "w"], "Fixed"),
    ("_integer2", "Integer2", &["x", "y"], "Integer"),
    ("_integer3", "Integer3", &["x", "y", "z"], "Integer"),
    ("_integer4", "Integer4", &["x", "y", "z", "w"], "Integer"),
];

/// The `_<element><dim>` type suffix decoded to its constructor type name, field
/// names, and element type.
fn vector_op_shape(target: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    VECTOR_SHAPES
        .iter()
        .find(|(suffix, _, _, _)| target.ends_with(suffix))
        .map(|(_, type_name, fields, element)| (*type_name, *fields, *element))
}

/// The bare op name for a `#vector_<op>_<element><dim>` target (suffix stripped).
fn vector_op_name(target: &str) -> Option<&str> {
    let op = target.strip_prefix("#vector_")?;
    Some(
        VECTOR_SHAPES
            .iter()
            .find_map(|(suffix, _, _, _)| op.strip_suffix(suffix))
            .unwrap_or(op),
    )
}

/// Whether `op` (with `argc` args on a vector of `type_name`/`element`) is one of
/// the ops `try_inline_vector_op` rewrites. `scale`/`dot`/`cross` are pure
/// arithmetic and inline for **every** element type (their re-lowered `*`/`+`/`-`
/// trees are bit-identical to the FUNC body, including the integer overflow
/// checks). `lerp`/`lerp_unclamped`/`length`/`distance` stay Float-only: they use
/// `math::sqrt` or Float clamp constants, and the Fixed/Integer bodies differ
/// (software isqrt etc.), so those keep their FUNC path.
fn vector_op_inlinable(op: &str, argc: usize, type_name: &str, element: &str) -> bool {
    match (op, argc) {
        ("scale", 2) | ("dot", 2) => true,
        ("cross", 2) => type_name.ends_with('3'),
        ("lerp_unclamped", 3) | ("lerp", 3) | ("length", 1) | ("distance", 2) => element == "Float",
        _ => false,
    }
}

/// Whether a `vector::` op call with `target`/`args` will be inlined by
/// `try_inline_vector_op` (so a `Local` argument to it is read as lanes, never
/// materialized). Single source of truth shared with the promotion escape
/// analysis — must mirror the `try_inline_vector_op` gate exactly.
pub(super) fn vector_call_is_inlined(target: &str, args: &[NirValue]) -> bool {
    let Some((type_name, _, element)) = vector_op_shape(target) else {
        return false;
    };
    let Some(op) = vector_op_name(target) else {
        return false;
    };
    if !args.iter().all(is_reevaluation_safe) {
        return false;
    }
    vector_op_inlinable(op, args.len(), type_name, element)
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
    /// Whether `value` is a register-native vector carried by a side-table marker.
    pub(super) fn is_vector_native(value: &ValueResult) -> bool {
        value.location.starts_with(VECTOR_NATIVE_MARKER)
    }

    /// The per-lane scalar `Float` values of a register-native vector, if it is one.
    pub(super) fn vector_native_lanes(&self, value: &ValueResult) -> Option<Vec<ValueResult>> {
        self.vector_natives.get(&value.location).cloned()
    }

    /// Register `lanes` as an in-flight register-native `type_` vector and return a
    /// `ValueResult` carrying its marker location (no allocation).
    pub(super) fn make_vector_native(&mut self, type_: &str, lanes: Vec<ValueResult>) -> ValueResult {
        let marker = format!("{VECTOR_NATIVE_MARKER}{}", self.next_vector_native);
        self.next_vector_native += 1;
        self.vector_natives.insert(marker.clone(), lanes);
        ValueResult {
            type_: type_.to_string(),
            location: marker,
            text: format!("vecnative {type_}"),
        }
    }

    /// A field read of a register-native vector (a lane), if `target_value` is one.
    pub(super) fn vector_native_field(
        &self,
        target_value: &ValueResult,
        member: &str,
    ) -> Option<ValueResult> {
        let lanes = self.vector_native_lanes(target_value)?;
        let index = vector_field_index(member)?;
        lanes.get(index).cloned()
    }

    /// Materialize a register-native vector into its N×8-byte arena block, spilling
    /// each lane first (so the block build's `arena_alloc` cannot clobber a live
    /// lane register) and writing the fields with the record layout. Identity for a
    /// value that is not register-native — the single boundary choke point.
    pub(super) fn vector_value_as_block(
        &mut self,
        value: ValueResult,
    ) -> Result<ValueResult, String> {
        let Some(lanes) = self.vector_native_lanes(&value) else {
            return Ok(value);
        };
        let mut slots = Vec::with_capacity(lanes.len());
        for lane in lanes {
            let lane = self.materialize_float(lane)?;
            let slot = self.allocate_stack_object("vector_lane", 8);
            self.emit(abi::store_u64(&lane.location, abi::stack_pointer(), slot));
            slots.push(slot);
        }
        let register = self.emit_build_inlined_record(&value.type_, &slots)?;
        let block = ValueResult {
            type_: value.type_,
            location: register,
            text: value.text,
        };
        // The materialized block is a fresh, freeable-flat arena block — register
        // it as a statement-scope temp exactly as an eager `Constructor` result is
        // (a native skips that registration at production, since it had no block
        // then). An owner boundary (`lower_value_owned`) claims it; a borrow
        // boundary (a call arg, a container-copy) leaves it to be freed at
        // statement end. This is what keeps the lazy carrier's frees identical to
        // the eager path.
        let slot = self.allocate_stack_object("pending_temp", 8);
        self.emit(abi::store_u64(&block.location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: block.type_.clone(),
            slot,
            location: block.location.clone(),
        });
        Ok(block)
    }

    /// The combined storage/escape-boundary materialization: a register-native
    /// vector becomes its block; a `d`-native `Float` becomes its GPR bits; every
    /// other value is unchanged. Every site that stores a value as 8 bytes or
    /// passes it as an argument routes through here.
    pub(super) fn materialize_value(&mut self, value: ValueResult) -> Result<ValueResult, String> {
        if Self::is_vector_native(&value) {
            return self.vector_value_as_block(value);
        }
        self.materialize_float(value)
    }

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
        let Some((type_name, fields, element)) = vector_op_shape(target) else {
            return Ok(None);
        };
        let Some(op) = vector_op_name(target) else {
            return Ok(None);
        };

        // Only the recognized ops are inlined (scale/dot/cross for every element
        // type; lerp/length/distance for Float only); every operand must be
        // re-evaluation-safe (the field reads duplicate it per lane).
        if !args.iter().all(is_reevaluation_safe) {
            return Ok(None);
        }
        if !vector_op_inlinable(op, args.len(), type_name, element) {
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
            // lerp_unclamped: Float_N[ a.f + (b.f - a.f) * t ] — pure arithmetic.
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
            // lerp (clamped): Float_N[ a.f + (b.f - a.f) * clamp(t, 0, 1) ]. Matches
            // the FUNC body; `math::clamp` is inlined native codegen (min/max, no
            // call/alloc), so re-evaluating it per lane is cheap and gives the same
            // deterministic `tc`.
            ("lerp", 3) => {
                let (a, b, t) = (&args[0], &args[1], &args[2]);
                let clamped_t = || NirValue::Call {
                    target: "math.clamp".to_string(),
                    args: vec![
                        t.clone(),
                        NirValue::Const {
                            type_: "Float".to_string(),
                            value: "0.0".to_string(),
                        },
                        NirValue::Const {
                            type_: "Float".to_string(),
                            value: "1.0".to_string(),
                        },
                    ],
                    loc,
                };
                let lanes = fields
                    .iter()
                    .map(|f| {
                        let delta =
                            bin("-", Self::vector_field(b, f), Self::vector_field(a, f));
                        bin("+", Self::vector_field(a, f), bin("*", delta, clamped_t()))
                    })
                    .collect();
                build(self, lanes)?
            }
            // cross (3D, two args): the standard right-handed cross product. The 2D
            // (1-arg perpendicular) and 4D (3-arg) forms have different shapes and
            // are left to the FUNC.
            ("cross", 2) if type_name.ends_with('3') => {
                let (a, b) = (&args[0], &args[1]);
                let m = |v: &NirValue, f: &str| Self::vector_field(v, f);
                let lanes = vec![
                    bin("-", bin("*", m(a, "y"), m(b, "z")), bin("*", m(a, "z"), m(b, "y"))),
                    bin("-", bin("*", m(a, "z"), m(b, "x")), bin("*", m(a, "x"), m(b, "z"))),
                    bin("-", bin("*", m(a, "x"), m(b, "y")), bin("*", m(a, "y"), m(b, "x"))),
                ];
                build(self, lanes)?
            }
            // length: math::sqrt(v.f0*v.f0 + v.f1*v.f1 + ...) — a single expression
            // (matching the FUNC body exactly, so the sum is finiteness-observed as
            // the sqrt argument and the sqrt result is finite by the boundary
            // invariant).
            ("length", 1) => {
                let v = &args[0];
                let square = |f: &str| bin("*", Self::vector_field(v, f), Self::vector_field(v, f));
                let mut sum = square(fields[0]);
                for f in &fields[1..] {
                    sum = bin("+", sum, square(f));
                }
                let sqrt = NirValue::Call {
                    target: "math.sqrt".to_string(),
                    args: vec![sum],
                    loc,
                };
                self.lower_value(&sqrt)?
            }
            // distance: math::sqrt((a.f0-b.f0)^2 + ...). The FUNC binds each
            // difference to a LET; inlining re-evaluates the (deterministic)
            // subtraction per square, so the value is bit-identical (a subtraction
            // that overflows still traps `ErrFloatOverflow`, at the call site's
            // location rather than the FUNC's — the code is unchanged).
            ("distance", 2) => {
                let (a, b) = (&args[0], &args[1]);
                let sq_diff = |f: &str| {
                    let diff = bin("-", Self::vector_field(a, f), Self::vector_field(b, f));
                    bin("*", diff.clone(), diff)
                };
                let mut sum = sq_diff(fields[0]);
                for f in &fields[1..] {
                    sum = bin("+", sum, sq_diff(f));
                }
                let sqrt = NirValue::Call {
                    target: "math.sqrt".to_string(),
                    args: vec![sum],
                    loc,
                };
                self.lower_value(&sqrt)?
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
