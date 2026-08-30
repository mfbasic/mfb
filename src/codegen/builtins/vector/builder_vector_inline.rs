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
//!
//! This module also owns the **register-native vector carrier** (bug-332 F3),
//! which is a codegen-wide escape-boundary hook rather than an inlining detail:
//! `VECTOR_NATIVE_MARKER` tags a `ValueResult.location` whose lanes live in the
//! `vector_natives` side-table, and `is_vector_native` / `vector_native_lanes` /
//! `make_vector_native` / `materialize_value` are the construction and
//! boundary-materialization API. Every site that stores a small-vector value as 8
//! bytes or passes it as an argument routes through `materialize_value`, so the
//! carrier is load-bearing beyond the `vector::`-op inlining above.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
/// The un-encodable prefix marking a `ValueResult.location` as a register-native
/// vector whose lanes live in the `vector_natives` side-table. Chosen so it can
/// never be a physical register / vreg / stack slot: if one leaks to a GP or
/// store site it hard-errors at the encoder (fail-loud) instead of miscompiling.
pub(crate) const VECTOR_NATIVE_MARKER: &str = "%%vecnative:";

/// The lane count of a register-native small vector type (Float/Fixed/Integer),
/// or `None`. The carrier (construction, member reads, boundary materialization)
/// is element-type-agnostic — a lane is a scalar `Float`/`Fixed`/`Integer` value
/// stored as 8 bytes — so every `<Elem>N` type is register-native. Only the *op
/// inlining* is Float-only (Fixed/Integer ops keep their FUNC bodies).
pub(crate) fn vector_field_count(type_: &ParameterType) -> Option<usize> {
    // The nine shapes are declared nominals (`Float3`, `Integer2`, …), so this
    // is a nominal identification, not a re-parse of a grammar: a non-`Named`
    // type is not a vector and answers `None` through the wildcard.
    let ParameterType::Named(name) = type_ else {
        return None;
    };
    match name.resolve() {
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
const VECTOR_SHAPES: &[(&str, &str, &[&str], ParameterType)] = &[
    ("_float2", "Float2", &["x", "y"], ParameterType::Float),
    ("_float3", "Float3", &["x", "y", "z"], ParameterType::Float),
    (
        "_float4",
        "Float4",
        &["x", "y", "z", "w"],
        ParameterType::Float,
    ),
    ("_fixed2", "Fixed2", &["x", "y"], ParameterType::Fixed),
    ("_fixed3", "Fixed3", &["x", "y", "z"], ParameterType::Fixed),
    (
        "_fixed4",
        "Fixed4",
        &["x", "y", "z", "w"],
        ParameterType::Fixed,
    ),
    ("_integer2", "Integer2", &["x", "y"], ParameterType::Integer),
    (
        "_integer3",
        "Integer3",
        &["x", "y", "z"],
        ParameterType::Integer,
    ),
    (
        "_integer4",
        "Integer4",
        &["x", "y", "z", "w"],
        ParameterType::Integer,
    ),
];

/// The `_<element><dim>` type suffix decoded to its constructor type name, field
/// names, and element type.
fn vector_op_shape(
    target: &str,
) -> Option<(
    &'static str,
    &'static [&'static str],
    &'static ParameterType,
)> {
    VECTOR_SHAPES
        .iter()
        .find(|(suffix, _, _, _)| target.ends_with(suffix))
        .map(|(_, type_name, fields, element)| (*type_name, *fields, element))
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
fn vector_op_inlinable(op: &str, argc: usize, fields: &[&str], element: &ParameterType) -> bool {
    match (op, argc) {
        ("scale", 2) | ("dot", 2) => true,
        // `cross` is 3-lane only. Reading the lane count off `fields` says that
        // directly; the old `type_name.ends_with('3')` inferred it from the last
        // character of the nominal's spelling.
        ("cross", 2) => fields.len() == 3,
        // plan-86 H2: length/distance inline for every numeric element — the sum of
        // squares is `bin(*/+)` (works for Integer/Fixed with their overflow checks),
        // and the sqrt is the same deterministic helper the FUNC body calls (Float/
        // Fixed → `math.sqrt` → `emit_fixed_sqrt`; Integer → `#vector_isqrtRound`), so
        // the inlined result is bit-identical while the operand N×8 block-materialize
        // is skipped. `lerp`/`lerp_unclamped` STAY Float-only: their Fixed/Integer
        // FUNC bodies do `toFixed`/`toFloat`/`round` conversions a pure arithmetic
        // tree would not reproduce.
        ("lerp_unclamped", 3) | ("lerp", 3) => *element == ParameterType::Float,
        ("length", 1) | ("distance", 2) => matches!(
            element,
            ParameterType::Float | ParameterType::Fixed | ParameterType::Integer
        ),
        // plan-86 H1: normalize inlines for Float ONLY. The Float body is
        // `len = sqrt(Σf²); IF len=0 THEN FAIL error(77050002); RETURN N[f/len,…]`
        // — a guarded per-lane divide that `inline_vector_normalize` reproduces.
        // Fixed/Integer normalize use the rounding integer sqrt + `toFixed`/`round`
        // conversions (a pure divide tree would not reproduce them), so they keep
        // their FUNC path.
        ("normalize", 1) => *element == ParameterType::Float,
        _ => false,
    }
}

/// Whether a `vector::` op call with `target`/`args` will be inlined by
/// `try_inline_vector_op` (so a `Local` argument to it is read as lanes, never
/// materialized). Single source of truth shared with the promotion escape
/// analysis — must mirror the `try_inline_vector_op` gate exactly.
pub(crate) fn vector_call_is_inlined(target: &str, args: &[NirValue]) -> bool {
    let Some((_, fields, element)) = vector_op_shape(target) else {
        return false;
    };
    let Some(op) = vector_op_name(target) else {
        return false;
    };
    if !args.iter().all(is_reevaluation_safe) {
        return false;
    }
    vector_op_inlinable(op, args.len(), fields, element)
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
    pub(crate) fn is_vector_native(value: &ValueResult) -> bool {
        value.location.render().starts_with(VECTOR_NATIVE_MARKER)
    }

    /// The per-lane scalar `Float` values of a register-native vector, if it is one.
    pub(crate) fn vector_native_lanes(&self, value: &ValueResult) -> Option<Vec<ValueResult>> {
        self.vector_natives.get(&value.location.render()).cloned()
    }

    /// Register `lanes` as an in-flight register-native `type_` vector and return a
    /// `ValueResult` carrying its marker location (no allocation).
    pub(crate) fn make_vector_native(
        &mut self,
        type_: &ParameterType,
        lanes: Vec<ValueResult>,
    ) -> ValueResult {
        let marker = format!("{VECTOR_NATIVE_MARKER}{}", self.next_vector_native);
        self.next_vector_native += 1;
        self.vector_natives.insert(marker.clone(), lanes);
        ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(marker),
            text: format!("vecnative {type_}"),
        }
    }

    /// A field read of a register-native vector (a lane), if `target_value` is one.
    pub(crate) fn vector_native_field(
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
    pub(crate) fn vector_value_as_block(
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
        let register = self.emit_build_inlined_record(&value.type_.name(), &slots)?;
        let block = ValueResult {
            origin: None,
            type_: value.type_,
            location: Operand::from(register.render()),
            text: value.text,
        };
        // The materialized block is a fresh, freeable-flat arena block — register
        // it as a statement-scope temp exactly as an eager `Constructor` result is
        // (a native skips that registration at production, since it had no block
        // then). An owner boundary (`lower_value_owned`) claims it; an alias
        // boundary (a call arg, a container-copy) leaves it to be freed at
        // statement end. This is what keeps the lazy carrier's frees identical to
        // the eager path.
        let slot = self.allocate_stack_object("pending_temp", 8);
        self.emit(abi::store_u64(&block.location, abi::stack_pointer(), slot));
        self.pending_temp_frees.push(PendingTemp {
            type_: block.type_.name().into_owned(),
            slot,
            location: block.location.clone(),
        });
        Ok(block)
    }

    /// The combined storage/escape-boundary materialization: a register-native
    /// vector becomes its block; a `d`-native `Float` becomes its GPR bits; every
    /// other value is unchanged. Every site that stores a value as 8 bytes or
    /// passes it as an argument routes through here.
    pub(crate) fn materialize_value(&mut self, value: ValueResult) -> Result<ValueResult, String> {
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
    /// plan-86 H2: the sqrt call `length`/`distance` inlines for an element type.
    /// Float/Fixed use `math::sqrt` (Fixed lowers to the deterministic
    /// `emit_fixed_sqrt`); Integer uses the package's rounding integer sqrt
    /// `#vector_isqrtRound` — the exact helper the Integer FUNC body calls, so the
    /// inlined result is bit-identical.
    fn vector_sqrt_of(element: &ParameterType, sum: NirValue, loc: NirSourceLoc) -> NirValue {
        let target = if *element == ParameterType::Integer {
            "#vector_isqrtRound"
        } else {
            "math.sqrt"
        };
        NirValue::Call {
            target: target.to_string(),
            args: vec![sum],
            loc,
        }
    }

    /// plan-86 H1: inline `vector::normalize` for a Float vector. Reproduces the
    /// FUNC body `len = math::sqrt(Σf²); IF len = 0.0 THEN FAIL error(77050002);
    /// RETURN Float_N[f/len, …]` — `len` is computed ONCE (bound to a synthetic
    /// Float local so the per-lane divides reuse it, exactly as the FUNC's `LET
    /// len`), guarded against zero with the same error code/message the FUNC emits,
    /// then each lane divides. Bit-identical to the FUNC on non-zero inputs; a
    /// zero-length vector still traps `7-705-0002` with the same message (the
    /// source location moves to the call site, which is not observable — the error
    /// output carries only code + message, per `rt-error/vector/normalize_zero_rt`).
    fn inline_vector_normalize(
        &mut self,
        v: &NirValue,
        type_: &ParameterType,
        fields: &[&str],
        loc: NirSourceLoc,
    ) -> Result<ValueResult, String> {
        let bin = |op: &str, left: NirValue, right: NirValue| NirValue::Binary {
            op: op.to_string(),
            left: Box::new(left),
            right: Box::new(right),
            loc,
        };
        // sum = f0*f0 + f1*f1 + … (left-associative, matching the FUNC).
        let square = |f: &str| bin("*", Self::vector_field(v, f), Self::vector_field(v, f));
        let mut sum = square(fields[0]);
        for f in &fields[1..] {
            sum = bin("+", sum, square(f));
        }
        // len = math::sqrt(sum), observed finite exactly as the FUNC's `LET len`.
        let len_node = NirValue::Call {
            target: "math.sqrt".to_string(),
            args: vec![sum],
            loc,
        };
        let len = self.lower_value(&len_node)?;
        self.observe_float(&len_node, &len)?;
        let len = self.materialize_value(len)?;
        let len_slot = self.allocate_stack_object("vecnorm_len", 8);
        self.store_value_at(&len, abi::stack_pointer(), len_slot);
        // Bind `len` as a synthetic Float local so the divide tree can read it back
        // (a re-evaluation-safe named binding); restore any shadowed local after.
        let len_name = "$vecnorm_len".to_string();
        let previous = self.locals.insert(
            len_name.clone(),
            LocalValue {
                type_: ParameterType::Float,
                stack_offset: len_slot,
                constant: None,
                by_ref: false,
            },
        );
        // IF len = 0.0 THEN FAIL error(77050002, "…zero-length…").
        let is_zero = self.lower_value(&bin(
            "=",
            NirValue::Local(len_name.clone()),
            NirValue::Const {
                type_: ParameterType::Float,
                value: "0.0".to_string(),
            },
        ))?;
        let ok_label = self.label("vecnorm_ok");
        self.emit(abi::compare_immediate(&is_zero.location, "0"));
        // is_zero == 0 (false → len ≠ 0) takes the normal path; otherwise fall
        // through into the terminal error return (branches to the function exit).
        self.emit(abi::branch_eq(&ok_label));
        self.emit_error_code_return("77050002", "vector::normalize of a zero-length vector")?;
        self.emit(abi::label(&ok_label));
        // RETURN Float_N[f0/len, f1/len, …].
        let lanes = fields
            .iter()
            .map(|f| {
                bin(
                    "/",
                    Self::vector_field(v, f),
                    NirValue::Local(len_name.clone()),
                )
            })
            .collect();
        let result = self.lower_value(&NirValue::Constructor {
            type_: type_.clone(),
            args: lanes,
        });
        match previous {
            Some(prev) => {
                self.locals.insert(len_name, prev);
            }
            None => {
                self.locals.remove(&len_name);
            }
        }
        result
    }

    pub(crate) fn try_inline_vector_op(
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
        if !vector_op_inlinable(op, args.len(), fields, element) {
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
                type_: ParameterType::named(type_name),
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
                        let delta = bin("-", Self::vector_field(b, f), Self::vector_field(a, f));
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
                            type_: ParameterType::Float,
                            value: "0.0".to_string(),
                        },
                        NirValue::Const {
                            type_: ParameterType::Float,
                            value: "1.0".to_string(),
                        },
                    ],
                    loc,
                };
                let lanes = fields
                    .iter()
                    .map(|f| {
                        let delta = bin("-", Self::vector_field(b, f), Self::vector_field(a, f));
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
                    bin(
                        "-",
                        bin("*", m(a, "y"), m(b, "z")),
                        bin("*", m(a, "z"), m(b, "y")),
                    ),
                    bin(
                        "-",
                        bin("*", m(a, "z"), m(b, "x")),
                        bin("*", m(a, "x"), m(b, "z")),
                    ),
                    bin(
                        "-",
                        bin("*", m(a, "x"), m(b, "y")),
                        bin("*", m(a, "y"), m(b, "x")),
                    ),
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
                self.lower_value(&Self::vector_sqrt_of(element, sum, loc))?
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
                self.lower_value(&Self::vector_sqrt_of(element, sum, loc))?
            }
            // normalize (Float): len = sqrt(Σf²); IF len=0 THEN FAIL; N[f/len,…].
            // Needs a guarded compare + FAIL between computing `len` and the divides,
            // so it cannot be a pure expression tree — factored into a helper.
            ("normalize", 1) => {
                // `VECTOR_SHAPES` stores the constructor nominal as a `&'static
                // str` because a `Named` carries a `Symbol` and is not
                // const-constructible; the nominal is built once, here.
                let type_ = ParameterType::named(type_name);
                self.inline_vector_normalize(&args[0], &type_, fields, loc)?
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
