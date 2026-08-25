//! `vector::lerp_unclamped` — descriptor entry + the per-type `__vector_lerp_unclamped_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str =
    r#"Linear interpolation between two vectors, extrapolating outside 0 through 1"#;

const DESC: &str = r#"`vector::lerp_unclamped` computes `a + (b - a) * t` component-wise in declared
field order, using `t` **verbatim** with no clamping. It is otherwise identical to
`vector::lerp` and shares its per-element-type behavior; the only difference
between the two implementations is the missing `math::clamp` call on `t`.

That single difference changes what the function is for. Because `t` is not
restricted to `0` through `1`, values outside that range **extrapolate** along the
infinite line through `a` and `b` rather than saturating at an endpoint: `t = 2.0`
lands as far beyond `b` as `b` is beyond `a`, and `t = -1.0` lands the same
distance before `a`. Use this when the parameter legitimately runs past the
endpoints — projecting a trajectory forward, or overshooting deliberately for an
easing effect — and use `vector::lerp` when an out-of-range `t` should be treated
as a mistake and pinned to the segment.

Extrapolation is also where this function's failure modes come from. Since `t` is
unbounded, so is the result: a large `t` scales the difference `b - a` without
limit and can drive a component past the range of the element type, which the
clamped `vector::lerp` cannot do for finite endpoints. On the `Integer` overloads
this surfaces as `ErrOverflow` from the final rounding back to `Integer`.

As with `vector::lerp`, `t` is a `Float` for **every** overload, including the
`Fixed` and `Integer` ones. The `Fixed` overloads convert `t` with `toFixed` and
interpolate in Q32.32; the `Integer` overloads widen each component to `Float`,
interpolate there, and round back with `math::round`, half away from zero.
`vector::slerp` falls back to this function, not to `vector::lerp`, when its two
inputs are too nearly parallel for the spherical formula to be stable — which is
why an out-of-range `t` passed to `slerp` still extrapolates in that degenerate
case.

`vector::lerp_unclamped` is generic over the nine built-in vector record types.
The first two arguments must be the *same* one of the nine types, and the third
must be a `Float` for every overload — an `Integer` `t` is a compile-time error
with no implicit numeric promotion. The return type is always the first argument's
own type."#;

const EX: &str = r#"Extrapolate twice as far as the endpoint:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp_unclamped(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 2.0)))
END SUB
```

Extrapolate backwards, before the start point:

```
IMPORT vector
IMPORT io

SUB main()
  LET back AS vector::Float2 = vector::lerp_unclamped(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 0.0 - 0.5)
  io::print(toString(back))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_lerp_unclamped_float2(a AS Float2, b AS Float2, t AS Float) AS Float2
  LET cx AS Float = a.x + (b.x - a.x) * t
  LET cy AS Float = a.y + (b.y - a.y) * t
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_lerp_unclamped_float3(a AS Float3, b AS Float3, t AS Float) AS Float3
  LET cx AS Float = a.x + (b.x - a.x) * t
  LET cy AS Float = a.y + (b.y - a.y) * t
  LET cz AS Float = a.z + (b.z - a.z) * t
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_lerp_unclamped_float4(a AS Float4, b AS Float4, t AS Float) AS Float4
  LET cx AS Float = a.x + (b.x - a.x) * t
  LET cy AS Float = a.y + (b.y - a.y) * t
  LET cz AS Float = a.z + (b.z - a.z) * t
  LET cw AS Float = a.w + (b.w - a.w) * t
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_lerp_unclamped_fixed2(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
  LET tf AS Fixed = toFixed(t)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_lerp_unclamped_fixed3(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
  LET tf AS Fixed = toFixed(t)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  LET cz AS Fixed = a.z + (b.z - a.z) * tf
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_lerp_unclamped_fixed4(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
  LET tf AS Fixed = toFixed(t)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  LET cz AS Fixed = a.z + (b.z - a.z) * tf
  LET cw AS Fixed = a.w + (b.w - a.w) * tf
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_lerp_unclamped_integer2(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * t)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * t)
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_lerp_unclamped_integer3(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * t)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * t)
  LET cz AS Integer = math::round(toFloat(a.z) + (toFloat(b.z) - toFloat(a.z)) * t)
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_lerp_unclamped_integer4(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * t)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * t)
  LET cz AS Integer = math::round(toFloat(a.z) + (toFloat(b.z) - toFloat(a.z)) * t)
  LET cw AS Integer = math::round(toFloat(a.w) + (toFloat(b.w) - toFloat(a.w)) * t)
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_lerp_unclamped_<type>` body for one applicable vector type.
fn body(ty: &str) -> &'static str {
    match ty {
        "Float2" => BODY_FLOAT2,
        "Float3" => BODY_FLOAT3,
        "Float4" => BODY_FLOAT4,
        "Fixed2" => BODY_FIXED2,
        "Fixed3" => BODY_FIXED3,
        "Fixed4" => BODY_FIXED4,
        "Integer2" => BODY_INTEGER2,
        "Integer3" => BODY_INTEGER3,
        "Integer4" => BODY_INTEGER4,
        other => unreachable!("vector::lerp_unclamped has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lerp_unclamped",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type and a Float t"),
        internal_only: false,
        implementations: super::implementations("lerp_unclamped", super::Shape::Lerp, &[], body),
    });
}
