//! `vector::angle` — descriptor entry + the per-type `__vector_angle_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Unsigned angle in radians between two vectors"#;

const DESC: &str = r#"`vector::angle` returns the unsigned angle between the directions of `a` and `b`,
**in radians**, computed as `acos(clamp(dot(a, b) / (length(a) * length(b)), -1, 1))`.
The result lies in `0` through `pi`: it is `0` for vectors pointing the same way
and `pi` for vectors pointing opposite ways. The angle is unsigned and symmetric —
`angle(a, b)` equals `angle(b, a)` — so it carries no orientation or handedness
information and cannot distinguish a clockwise from a counterclockwise
separation. Magnitude is irrelevant: scaling either input by a positive factor
leaves the result unchanged.

The cosine is clamped to the closed interval `-1` through `1` with `math::clamp`
before `acos` is applied. This matters because the division can produce a value a
fraction of an ulp outside that interval for nearly parallel or nearly
antiparallel inputs; without the clamp `acos` would fail with a domain error. With
it, the function is total for every pair of non-zero inputs and never raises a
floating-point domain error.

Both inputs must be non-zero. A zero-length vector has no direction, so the
implementation checks each length before dividing and fails with
`ErrInvalidArgument` and the message `vector::angle with a zero-length vector` if
either is zero. The check is on the actual computed length, so the failure
happens before any division by zero can occur.

The `Integer` overloads are the coarsest. They compute the angle internally in
`Fixed` (Q32.32) radians through a dedicated helper, then round that radian value
to an `Integer` with `math::round`, half away from zero. Because the full range of
the function is `0` through `pi`, the only possible `Integer` results are `0`,
`1`, `2`, and `3`. The `Integer` overload is therefore a very lossy quantization
of the angle and is rarely the right tool; prefer the `Float` or `Fixed`
overloads when the angle itself matters.

`vector::angle` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself."#;

const EX: &str = r#"The right angle between the two 2D axes, in radians:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::angle(vector::Float2[1.0, 0.0], vector::Float2[0.0, 1.0])))
END SUB
```

The angle is unaffected by magnitude:

```
IMPORT vector
IMPORT io

SUB main()
  LET wide AS Float = vector::angle(vector::Float3[10.0, 0.0, 0.0], vector::Float3[0.0, 7.0, 0.0])
  io::print(toString(wide))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_angle_float2(a AS Float2, b AS Float2) AS Float
  LET la AS Float = math::sqrt(__vector_dot_float2(a, a))
  LET lb AS Float = math::sqrt(__vector_dot_float2(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Float = __vector_dot_float2(a, b) / (la * lb)
  LET clamped AS Float = math::clamp(cosv, -1.0, 1.0)
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_angle_float3(a AS Float3, b AS Float3) AS Float
  LET la AS Float = math::sqrt(__vector_dot_float3(a, a))
  LET lb AS Float = math::sqrt(__vector_dot_float3(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Float = __vector_dot_float3(a, b) / (la * lb)
  LET clamped AS Float = math::clamp(cosv, -1.0, 1.0)
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_angle_float4(a AS Float4, b AS Float4) AS Float
  LET la AS Float = math::sqrt(__vector_dot_float4(a, a))
  LET lb AS Float = math::sqrt(__vector_dot_float4(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Float = __vector_dot_float4(a, b) / (la * lb)
  LET clamped AS Float = math::clamp(cosv, -1.0, 1.0)
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_angle_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed
  LET la AS Fixed = math::sqrt(__vector_dot_fixed2(a, a))
  LET lb AS Fixed = math::sqrt(__vector_dot_fixed2(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Fixed = __vector_dot_fixed2(a, b) / (la * lb)
  LET clamped AS Fixed = math::clamp(cosv, toFixed(-1.0), toFixed(1.0))
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_angle_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed
  LET la AS Fixed = math::sqrt(__vector_dot_fixed3(a, a))
  LET lb AS Fixed = math::sqrt(__vector_dot_fixed3(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Fixed = __vector_dot_fixed3(a, b) / (la * lb)
  LET clamped AS Fixed = math::clamp(cosv, toFixed(-1.0), toFixed(1.0))
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_angle_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed
  LET la AS Fixed = math::sqrt(__vector_dot_fixed4(a, a))
  LET lb AS Fixed = math::sqrt(__vector_dot_fixed4(b, b))
  IF la = 0.0 OR lb = 0.0 THEN
    FAIL error(77050002, "vector::angle with a zero-length vector")
  END IF
  LET cosv AS Fixed = __vector_dot_fixed4(a, b) / (la * lb)
  LET clamped AS Fixed = math::clamp(cosv, toFixed(-1.0), toFixed(1.0))
  RETURN math::acos(clamped)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_angle_integer2(a AS Integer2, b AS Integer2) AS Integer
  RETURN math::round(__vector_angleFixed_integer2(a, b))
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_angle_integer3(a AS Integer3, b AS Integer3) AS Integer
  RETURN math::round(__vector_angleFixed_integer3(a, b))
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_angle_integer4(a AS Integer4, b AS Integer4) AS Integer
  RETURN math::round(__vector_angleFixed_integer4(a, b))
END FUNC"#;

/// The `__vector_angle_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::angle has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "angle",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations(
            "angle",
            super::Shape::BinaryScalar,
            &["ErrInvalidArgument"],
            body,
            &[
                "The first vector. Must not be zero-length — a zero vector has no direction to measure from.",
                "The second vector. Must not be zero-length either.",
            ],
        ),
    });
}
