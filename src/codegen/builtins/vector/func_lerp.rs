//! `vector::lerp` — descriptor entry + the per-type `__vector_lerp_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Linear interpolation between two vectors, clamped to the segment"#;

const DESC: &str = r#"`vector::lerp` interpolates component-wise along the straight segment from `a` to
`b`, computing `a + (b - a) * t` for each component in declared field order. At
`t = 0` the result is `a`, at `t = 1` it is `b`, and at `t = 0.5` it is the
midpoint. The path traced as `t` sweeps is a straight line, and the speed along
it is constant — for interpolation that follows the arc between two directions
instead, use `vector::slerp`.

The defining difference from `vector::lerp_unclamped` is that `t` is **clamped to
the closed interval 0 through 1** with `math::clamp` before it is used. A `t` of
`2.0` therefore behaves exactly like `1.0` and returns `b`; a `t` of `-1.0`
behaves like `0.0` and returns `a`. The result is guaranteed to lie on the segment
between the two endpoints and can never overshoot them, which makes `lerp` the
safe choice when `t` comes from a source that may run past its expected range,
such as an elapsed-time ratio.

`t` is a `Float` for **every** overload, including the `Fixed` and `Integer`
ones — it is not the vector's element type. This differs from
`vector::clamp_length`, whose scalar argument does follow the element type. On
the `Fixed` overloads the clamped `t` is converted with `toFixed` after the clamp,
and the interpolation then runs entirely in Q32.32. On the `Integer` overloads
each component is widened to `Float`, interpolated there, and rounded back with
`math::round`, half away from zero — so `lerp` on `Integer` vectors quantizes the
result to the integer lattice, and successive small steps of `t` can produce the
same output repeatedly.

Interpolation is strictly component-wise, so `lerp` preserves neither length nor
direction in general: the midpoint of two unit vectors pointing different ways is
shorter than either, because it cuts across the chord rather than following the
arc.

`vector::lerp` is generic over the nine built-in vector record types. The first
two arguments must be the *same* one of the nine types, and the third must be a
`Float` for every overload — an `Integer` `t` is a compile-time error with no
implicit numeric promotion. The return type is always the first argument's own
type."#;

const EX: &str = r#"The midpoint of a segment:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 0.5)))
END SUB
```

An out-of-range `t` is clamped, so this returns the endpoint rather than
overshooting it:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::lerp(vector::Float2[0.0, 0.0], vector::Float2[10.0, 0.0], 2.0)))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_lerp_float2(a AS Float2, b AS Float2, t AS Float) AS Float2
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Float = a.x + (b.x - a.x) * tc
  LET cy AS Float = a.y + (b.y - a.y) * tc
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_lerp_float3(a AS Float3, b AS Float3, t AS Float) AS Float3
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Float = a.x + (b.x - a.x) * tc
  LET cy AS Float = a.y + (b.y - a.y) * tc
  LET cz AS Float = a.z + (b.z - a.z) * tc
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_lerp_float4(a AS Float4, b AS Float4, t AS Float) AS Float4
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Float = a.x + (b.x - a.x) * tc
  LET cy AS Float = a.y + (b.y - a.y) * tc
  LET cz AS Float = a.z + (b.z - a.z) * tc
  LET cw AS Float = a.w + (b.w - a.w) * tc
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_lerp_fixed2(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET tf AS Fixed = toFixed(tc)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_lerp_fixed3(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET tf AS Fixed = toFixed(tc)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  LET cz AS Fixed = a.z + (b.z - a.z) * tf
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_lerp_fixed4(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET tf AS Fixed = toFixed(tc)
  LET cx AS Fixed = a.x + (b.x - a.x) * tf
  LET cy AS Fixed = a.y + (b.y - a.y) * tf
  LET cz AS Fixed = a.z + (b.z - a.z) * tf
  LET cw AS Fixed = a.w + (b.w - a.w) * tf
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_lerp_integer2(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * tc)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * tc)
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_lerp_integer3(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * tc)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * tc)
  LET cz AS Integer = math::round(toFloat(a.z) + (toFloat(b.z) - toFloat(a.z)) * tc)
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_lerp_integer4(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
  LET tc AS Float = math::clamp(t, 0.0, 1.0)
  LET cx AS Integer = math::round(toFloat(a.x) + (toFloat(b.x) - toFloat(a.x)) * tc)
  LET cy AS Integer = math::round(toFloat(a.y) + (toFloat(b.y) - toFloat(a.y)) * tc)
  LET cz AS Integer = math::round(toFloat(a.z) + (toFloat(b.z) - toFloat(a.z)) * tc)
  LET cw AS Integer = math::round(toFloat(a.w) + (toFloat(b.w) - toFloat(a.w)) * tc)
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_lerp_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::lerp has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "lerp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type and a Float t"),
        internal_only: false,
        implementations: super::implementations("lerp", super::Shape::Lerp, &[], body, &[
            "The start vector, returned when `t` is 0.",
            "The end vector, returned when `t` is 1.",
            "How far along to travel, 0 through 1. Values outside that range are clamped — use `vector::lerp_unclamped` to extrapolate.",
        ]),
    });
}
