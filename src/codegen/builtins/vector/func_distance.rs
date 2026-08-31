//! `vector::distance` — descriptor entry + the per-type `__vector_distance_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Euclidean distance between two points"#;

const DESC: &str = r#"`vector::distance` treats `a` and `b` as points and returns the straight-line
Euclidean distance between them: the square root of the sum of the squared
per-component differences, `sqrt((a.x-b.x)^2 + (a.y-b.y)^2 + ...)`. The result is
always non-negative and is symmetric in the arguments — `distance(a, b)` equals
`distance(b, a)`, because each difference is squared before it is summed.
`distance(a, a)` is zero for every input.

The differences are formed component by component into named locals first, in
declared field order, and only then squared and summed; the sum is accumulated
left to right, `x` before `y` before `z` before `w`. This fixed evaluation order
is what makes the result reproducible bit for bit across targets. The function is
mathematically equal to `vector::length` of the componentwise difference of the
two vectors, and shares its per-element-type behavior, but it is a distinct
implementation that never materializes that difference vector as a record.

The `Float` overloads take the square root with `math::sqrt` over IEEE doubles.
The `Fixed` overloads use the deterministic Q32.32 square root. The `Integer`
overloads square and sum in exact checked integer arithmetic and then apply the
package's rounding integer square root, which returns the nearest integer to the
true root with halves rounded away from zero — so an `Integer` distance is a
rounded distance, not a truncated one, and `distance(vector::Integer2[0,0], vector::Integer2[3,4])`
is exactly `5`.

Unlike `vector::normalize` or `vector::angle`, `distance` has no degenerate input
to reject: coincident points are a perfectly ordinary case returning zero. It
therefore never raises `ErrInvalidArgument`. It is not, however, error-free: the
squaring step is ordinary checked arithmetic in the element type and can overflow
for large coordinates, and on the `Integer` overloads the *difference* itself can
overflow before any squaring, when subtracting a large negative coordinate from a
large positive one.

`vector::distance` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself."#;

const EX: &str = r#"The distance across a 3-4-5 triangle:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::distance(vector::Float2[0.0, 0.0], vector::Float2[3.0, 4.0])))
END SUB
```

The same measurement in exact integer coordinates:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::distance(vector::Integer2[0, 0], vector::Integer2[3, 4])))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_distance_float2(a AS Float2, b AS Float2) AS Float
  LET dx AS Float = a.x - b.x
  LET dy AS Float = a.y - b.y
  RETURN math::sqrt(dx * dx + dy * dy)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_distance_float3(a AS Float3, b AS Float3) AS Float
  LET dx AS Float = a.x - b.x
  LET dy AS Float = a.y - b.y
  LET dz AS Float = a.z - b.z
  RETURN math::sqrt(dx * dx + dy * dy + dz * dz)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_distance_float4(a AS Float4, b AS Float4) AS Float
  LET dx AS Float = a.x - b.x
  LET dy AS Float = a.y - b.y
  LET dz AS Float = a.z - b.z
  LET dw AS Float = a.w - b.w
  RETURN math::sqrt(dx * dx + dy * dy + dz * dz + dw * dw)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_distance_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed
  LET dx AS Fixed = a.x - b.x
  LET dy AS Fixed = a.y - b.y
  RETURN math::sqrt(dx * dx + dy * dy)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_distance_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed
  LET dx AS Fixed = a.x - b.x
  LET dy AS Fixed = a.y - b.y
  LET dz AS Fixed = a.z - b.z
  RETURN math::sqrt(dx * dx + dy * dy + dz * dz)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_distance_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed
  LET dx AS Fixed = a.x - b.x
  LET dy AS Fixed = a.y - b.y
  LET dz AS Fixed = a.z - b.z
  LET dw AS Fixed = a.w - b.w
  RETURN math::sqrt(dx * dx + dy * dy + dz * dz + dw * dw)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_distance_integer2(a AS Integer2, b AS Integer2) AS Integer
  LET dx AS Integer = a.x - b.x
  LET dy AS Integer = a.y - b.y
  LET s AS Integer = dx * dx + dy * dy
  RETURN __vector_isqrtRound(s)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_distance_integer3(a AS Integer3, b AS Integer3) AS Integer
  LET dx AS Integer = a.x - b.x
  LET dy AS Integer = a.y - b.y
  LET dz AS Integer = a.z - b.z
  LET s AS Integer = dx * dx + dy * dy + dz * dz
  RETURN __vector_isqrtRound(s)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_distance_integer4(a AS Integer4, b AS Integer4) AS Integer
  LET dx AS Integer = a.x - b.x
  LET dy AS Integer = a.y - b.y
  LET dz AS Integer = a.z - b.z
  LET dw AS Integer = a.w - b.w
  LET s AS Integer = dx * dx + dy * dy + dz * dz + dw * dw
  RETURN __vector_isqrtRound(s)
END FUNC"#;

/// The `__vector_distance_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::distance has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "distance",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations(
            "distance",
            super::Shape::BinaryScalar,
            &[],
            body,
            &[
                "The first point.",
                "The second point. Distance is symmetric, so the order does not matter.",
            ],
        ),
    });
}
