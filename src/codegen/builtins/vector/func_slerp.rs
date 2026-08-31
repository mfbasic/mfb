//! `vector::slerp` — descriptor entry + the per-type `__vector_slerp_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Spherical linear interpolation along the arc between two vectors"#;

const DESC: &str = r#"`vector::slerp` interpolates along the great-circle arc between the directions of
`a` and `b` rather than along the straight chord between their tips. It first
computes `omega = vector::angle(a, b)` and `s = sin(omega)`, then returns
`(sin((1-t)*omega)/s) * a + (sin(t*omega)/s) * b`. The result sweeps the angle
between the two inputs at a constant angular rate, which is what makes `slerp` the
right choice for interpolating orientations and directions where
`vector::lerp` would slow down in the middle of the turn.

**`t` is not clamped.** Values below `0` or above `1` extrapolate along the same
great circle, past either endpoint, exactly as `vector::lerp_unclamped` does along
its line. Clamp `t` yourself if that is not wanted.

`slerp` interpolates *direction*, and it preserves magnitude only when
`vector::length(a)` equals `vector::length(b)`. The two weights are derived purely
from the angle, so for inputs of different lengths the intermediate magnitudes
follow the weighted blend rather than tracking a sphere. For a clean directional
interpolation, normalize both inputs first.

The formula divides by `sin(omega)`, which approaches zero as the inputs become
parallel or antiparallel. To stay stable there, `slerp` tests
`abs(s) < 0.000001` — the literal threshold, in the `Float` overloads, and its
`toFixed` equivalent in the others — and when it is met **returns
`vector::lerp_unclamped(a, b, t)` instead**, taking the straight-line result. This
fallback is silent: nothing in the return value distinguishes the spherical path
from the linear one, and for nearly parallel inputs the two are in any case
indistinguishable. Note that the fallback is chosen for the *antiparallel* case as
well, where `sin(pi)` is also near zero; there is no unique great circle between
opposite directions, and `slerp` does not attempt to pick one — it interpolates
straight through the origin.

Both inputs must be non-zero. The requirement is inherited from
`vector::angle`, which is called first and fails with `ErrInvalidArgument` when
either input has zero length; the message therefore names `vector::angle` rather
than `vector::slerp`.

As with `vector::lerp`, `t` is a `Float` for **every** overload. The `Float`
overloads use the in-tree `Float` `sin`; the `Fixed` overloads work in
deterministic Q32.32 throughout. The `Integer` overloads compute the angle and the
weights in `Fixed`, blend the components there, and round each result back with
`math::round`, half away from zero, so an `Integer` `slerp` is heavily quantized —
its degenerate-case fallback goes to the `Integer` `lerp_unclamped`, which rounds
in the same way.

`vector::slerp` is generic over the nine built-in vector record types. The first
two arguments must be the *same* one of the nine types, and the third must be a
`Float` for every overload — an `Integer` `t` is a compile-time error with no
implicit numeric promotion. The return type is always the first argument's own
type."#;

const EX: &str = r#"Halfway along the arc between the two 2D axes — note that both components come
out equal, unlike the straight-line midpoint:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::slerp(vector::Float2[1.0, 0.0], vector::Float2[0.0, 1.0], 0.5)))
END SUB
```

Interpolating between two normalized directions keeps the result on the unit
circle:

```
IMPORT vector
IMPORT io

SUB main()
  LET start AS vector::Float3 = vector::normalize(vector::Float3[1.0, 0.0, 0.0])
  LET finish AS vector::Float3 = vector::normalize(vector::Float3[0.0, 0.0, 1.0])
  io::print(toString(vector::slerp(start, finish, 0.25)))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_slerp_float2(a AS Float2, b AS Float2, t AS Float) AS Float2
  LET omega AS Float = __vector_angle_float2(a, b)
  LET s AS Float = math::sin(omega)
  IF math::abs(s) < 0.000001 THEN
    RETURN __vector_lerp_unclamped_float2(a, b, t)
  END IF
  LET w0 AS Float = math::sin((1.0 - t) * omega) / s
  LET w1 AS Float = math::sin(t * omega) / s
  LET cx AS Float = w0 * a.x + w1 * b.x
  LET cy AS Float = w0 * a.y + w1 * b.y
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_slerp_float3(a AS Float3, b AS Float3, t AS Float) AS Float3
  LET omega AS Float = __vector_angle_float3(a, b)
  LET s AS Float = math::sin(omega)
  IF math::abs(s) < 0.000001 THEN
    RETURN __vector_lerp_unclamped_float3(a, b, t)
  END IF
  LET w0 AS Float = math::sin((1.0 - t) * omega) / s
  LET w1 AS Float = math::sin(t * omega) / s
  LET cx AS Float = w0 * a.x + w1 * b.x
  LET cy AS Float = w0 * a.y + w1 * b.y
  LET cz AS Float = w0 * a.z + w1 * b.z
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_slerp_float4(a AS Float4, b AS Float4, t AS Float) AS Float4
  LET omega AS Float = __vector_angle_float4(a, b)
  LET s AS Float = math::sin(omega)
  IF math::abs(s) < 0.000001 THEN
    RETURN __vector_lerp_unclamped_float4(a, b, t)
  END IF
  LET w0 AS Float = math::sin((1.0 - t) * omega) / s
  LET w1 AS Float = math::sin(t * omega) / s
  LET cx AS Float = w0 * a.x + w1 * b.x
  LET cy AS Float = w0 * a.y + w1 * b.y
  LET cz AS Float = w0 * a.z + w1 * b.z
  LET cw AS Float = w0 * a.w + w1 * b.w
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_slerp_fixed2(a AS Fixed2, b AS Fixed2, t AS Float) AS Fixed2
  LET omega AS Fixed = __vector_angle_fixed2(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_fixed2(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Fixed = w0 * a.x + w1 * b.x
  LET cy AS Fixed = w0 * a.y + w1 * b.y
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_slerp_fixed3(a AS Fixed3, b AS Fixed3, t AS Float) AS Fixed3
  LET omega AS Fixed = __vector_angle_fixed3(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_fixed3(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Fixed = w0 * a.x + w1 * b.x
  LET cy AS Fixed = w0 * a.y + w1 * b.y
  LET cz AS Fixed = w0 * a.z + w1 * b.z
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_slerp_fixed4(a AS Fixed4, b AS Fixed4, t AS Float) AS Fixed4
  LET omega AS Fixed = __vector_angle_fixed4(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_fixed4(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Fixed = w0 * a.x + w1 * b.x
  LET cy AS Fixed = w0 * a.y + w1 * b.y
  LET cz AS Fixed = w0 * a.z + w1 * b.z
  LET cw AS Fixed = w0 * a.w + w1 * b.w
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_slerp_integer2(a AS Integer2, b AS Integer2, t AS Float) AS Integer2
  LET omega AS Fixed = __vector_angleFixed_integer2(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_integer2(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Integer = math::round(w0 * toFixed(a.x) + w1 * toFixed(b.x))
  LET cy AS Integer = math::round(w0 * toFixed(a.y) + w1 * toFixed(b.y))
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_slerp_integer3(a AS Integer3, b AS Integer3, t AS Float) AS Integer3
  LET omega AS Fixed = __vector_angleFixed_integer3(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_integer3(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Integer = math::round(w0 * toFixed(a.x) + w1 * toFixed(b.x))
  LET cy AS Integer = math::round(w0 * toFixed(a.y) + w1 * toFixed(b.y))
  LET cz AS Integer = math::round(w0 * toFixed(a.z) + w1 * toFixed(b.z))
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_slerp_integer4(a AS Integer4, b AS Integer4, t AS Float) AS Integer4
  LET omega AS Fixed = __vector_angleFixed_integer4(a, b)
  LET s AS Fixed = math::sin(omega)
  IF math::abs(s) < toFixed(0.000001) THEN
    RETURN __vector_lerp_unclamped_integer4(a, b, t)
  END IF
  LET tf AS Fixed = toFixed(t)
  LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s
  LET w1 AS Fixed = math::sin(tf * omega) / s
  LET cx AS Integer = math::round(w0 * toFixed(a.x) + w1 * toFixed(b.x))
  LET cy AS Integer = math::round(w0 * toFixed(a.y) + w1 * toFixed(b.y))
  LET cz AS Integer = math::round(w0 * toFixed(a.z) + w1 * toFixed(b.z))
  LET cw AS Integer = math::round(w0 * toFixed(a.w) + w1 * toFixed(b.w))
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_slerp_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::slerp has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "slerp",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type and a Float t"),
        internal_only: false,
        implementations: super::implementations(
            "slerp",
            super::Shape::Lerp,
            &["ErrInvalidArgument"],
            body,
            &[
                "The start vector. Must not be zero-length — the interpolation is over directions.",
                "The end vector. Must not be zero-length either.",
                "How far along the arc to travel, 0 through 1.",
            ],
        ),
    });
}
