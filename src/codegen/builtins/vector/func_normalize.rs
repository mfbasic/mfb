//! `vector::normalize` — descriptor entry + the per-type `__vector_normalize_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Unit vector pointing the same way as the given vector"#;

const DESC: &str = r#"`vector::normalize` divides every component of `v` by `vector::length(v)`,
yielding a vector of magnitude `1` pointing in the same direction. The length is
computed once, in declared field order, and then each component is divided by it
in turn, so the direction is preserved and only the magnitude changes. The
argument is not modified — a fresh record is returned.

**A zero-length vector is rejected.** It has no direction, so there is no unit
vector to return, and dividing by its length would be a division by zero. It computes the length
first and fails with `ErrInvalidArgument` and
the message `vector::normalize of a zero-length vector` when it is zero, rather
than returning the zero vector or a vector of `NaN` components. This is a
deliberate contrast with `vector::clamp_length`, which accepts the zero vector and
passes it through unchanged. Callers that want a zero-safe normalize must test the
length themselves, or trap the error.

The `Float` and `Fixed` overloads divide with the correctly-rounded division of
their element type, giving a result whose length is `1` to within the precision of
that type. The `Integer` overloads are a different matter and are **intentionally
lossy**. They test the squared length for zero, take the rounding integer square
root of it, widen that integer length to `Float`, divide each component there, and
round each quotient back with `math::round`, half away from zero. Because a true
unit vector's components lie between `-1` and `1`, every rounded `Integer`
component collapses to `-1`, `0`, or `1`. `vector::normalize(vector::Integer2[3, 4])`, for
example, returns `(1, 1)` — the exact quotients `0.6` and `0.8` both round to `1`
— which is not a unit vector at all. The `Integer` overloads are best read as
"snap to the nearest lattice direction", and code that needs a real unit vector
should use the `Float` or `Fixed` overloads.

Note that the zero test differs slightly across element types: the `Float` and
`Fixed` overloads compare the computed square root against zero, while the
`Integer` overloads compare the squared sum against zero before taking any root.
Both reject exactly the all-zero vector.

`vector::normalize` is generic over the nine built-in vector record types. The
overload is selected at compile time from the exact record type of the single
argument; no implicit conversion or numeric promotion is applied to a vector
argument, and a non-vector argument or any arity other than one is rejected by the
syntax check. The return type is always the argument's own type."#;

const EX: &str = r#"Normalize a 3-4-5 vector to unit length:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::normalize(vector::Float3[3.0, 0.0, 4.0])))
END SUB
```

The `Integer` overload snaps to the nearest lattice direction rather than
producing a unit vector:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::normalize(vector::Integer2[3, 4])))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_normalize_float2(v AS Float2) AS Float2
  LET len AS Float = math::sqrt(v.x * v.x + v.y * v.y)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Float = v.x / len
  LET cy AS Float = v.y / len
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_normalize_float3(v AS Float3) AS Float3
  LET len AS Float = math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Float = v.x / len
  LET cy AS Float = v.y / len
  LET cz AS Float = v.z / len
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_normalize_float4(v AS Float4) AS Float4
  LET len AS Float = math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Float = v.x / len
  LET cy AS Float = v.y / len
  LET cz AS Float = v.z / len
  LET cw AS Float = v.w / len
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_normalize_fixed2(v AS Fixed2) AS Fixed2
  LET len AS Fixed = math::sqrt(v.x * v.x + v.y * v.y)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Fixed = v.x / len
  LET cy AS Fixed = v.y / len
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_normalize_fixed3(v AS Fixed3) AS Fixed3
  LET len AS Fixed = math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Fixed = v.x / len
  LET cy AS Fixed = v.y / len
  LET cz AS Fixed = v.z / len
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_normalize_fixed4(v AS Fixed4) AS Fixed4
  LET len AS Fixed = math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w)
  IF len = 0.0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET cx AS Fixed = v.x / len
  LET cy AS Fixed = v.y / len
  LET cz AS Fixed = v.z / len
  LET cw AS Fixed = v.w / len
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_normalize_integer2(v AS Integer2) AS Integer2
  LET s AS Integer = v.x * v.x + v.y * v.y
  IF s = 0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET len AS Integer = __vector_isqrtRound(s)
  LET fl AS Float = toFloat(len)
  LET rx AS Integer = math::round(toFloat(v.x) / fl)
  LET ry AS Integer = math::round(toFloat(v.y) / fl)
  RETURN Integer2[rx, ry]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_normalize_integer3(v AS Integer3) AS Integer3
  LET s AS Integer = v.x * v.x + v.y * v.y + v.z * v.z
  IF s = 0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET len AS Integer = __vector_isqrtRound(s)
  LET fl AS Float = toFloat(len)
  LET rx AS Integer = math::round(toFloat(v.x) / fl)
  LET ry AS Integer = math::round(toFloat(v.y) / fl)
  LET rz AS Integer = math::round(toFloat(v.z) / fl)
  RETURN Integer3[rx, ry, rz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_normalize_integer4(v AS Integer4) AS Integer4
  LET s AS Integer = v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w
  IF s = 0 THEN
    FAIL error(77050002, "vector::normalize of a zero-length vector")
  END IF
  LET len AS Integer = __vector_isqrtRound(s)
  LET fl AS Float = toFloat(len)
  LET rx AS Integer = math::round(toFloat(v.x) / fl)
  LET ry AS Integer = math::round(toFloat(v.y) / fl)
  LET rz AS Integer = math::round(toFloat(v.z) / fl)
  LET rw AS Integer = math::round(toFloat(v.w) / fl)
  RETURN Integer4[rx, ry, rz, rw]
END FUNC"#;

/// The `__vector_normalize_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::normalize has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "normalize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("a vector (Float2/3/4, Fixed2/3/4, Integer2/3/4)"),
        internal_only: false,
        implementations: super::implementations(
            "normalize",
            super::Shape::UnaryVector,
            &["ErrInvalidArgument"],
            body,
            &[
                "The vector to scale to unit length. A zero-length vector has no direction, so it is the one input this rejects.",
            ],
        ),
    });
}
