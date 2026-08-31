//! `vector::project` — descriptor entry + the per-type `__vector_project_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Vector projection of one vector onto another"#;

const DESC: &str = r#"`vector::project` returns the component of `a` that lies along `b`, computed as
`(dot(a, b) / dot(b, b)) * b`. The scalar ratio is formed once and then multiplies
each component of `b` in declared field order, so the result is always parallel to
`b` — a scalar multiple of it — and never has any component orthogonal to `b`.
Together with `vector::reject`, which returns the orthogonal remainder, it splits
`a` into two pieces that sum back to `a`.

The ratio's sign carries meaning: it is positive when `a` leans the same way as
`b`, zero when `a` is orthogonal to `b` (in which case the projection is the zero
vector), and negative when `a` leans against `b`, giving a projection that points
opposite to `b`. Note that only the *direction* of `b` matters for the result, not
its magnitude — the `dot(b, b)` in the denominator cancels the scaling — so
projecting onto `b` and onto `2 * b` gives the same answer.

**`b` must not be the zero vector.** `b`'s squared length is taken first
and, when it is zero, fails with `ErrInvalidArgument` and the message
`vector::project onto a zero-length vector` rather than dividing by zero. Note
that the guard is on the squared length rather than on the vector's components
directly; the two coincide for exact arithmetic, but on the `Fixed` overloads a
vector whose components are small enough that every square underflows to zero in
Q32.32 will also be rejected. `a`, by contrast, is unconstrained — the zero vector
is a perfectly ordinary `a` and projects to the zero vector.

The `Float` and `Fixed` overloads form the ratio and the products in their own
element type with correctly-rounded division. The `Integer` overloads are
**intentionally lossy**: the guard and the dot products are exact checked integer
arithmetic, but the ratio is computed in `Float` and each scaled component is
rounded back with `math::round`, half away from zero. An `Integer` projection is
therefore a lattice approximation, and the identity
`project(a, b) + reject(a, b) = a` still holds exactly only because
`vector::reject` is defined by subtracting the rounded projection from `a`.

`vector::project` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type."#;

const EX: &str = r#"Project a diagonal vector onto the `+x` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::project(vector::Float2[2.0, 2.0], vector::Float2[1.0, 0.0])))
END SUB
```

Projection and rejection sum back to the original vector:

```
IMPORT vector
IMPORT io

SUB main()
  LET a AS vector::Float3 = vector::Float3[2.0, 3.0, 4.0]
  LET b AS vector::Float3 = vector::Float3[0.0, 1.0, 0.0]
  io::print(toString(vector::project(a, b)))
  io::print(toString(vector::reject(a, b)))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_project_float2(a AS Float2, b AS Float2) AS Float2
  LET db AS Float = __vector_dot_float2(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = __vector_dot_float2(a, b) / db
  LET cx AS Float = ratio * b.x
  LET cy AS Float = ratio * b.y
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_project_float3(a AS Float3, b AS Float3) AS Float3
  LET db AS Float = __vector_dot_float3(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = __vector_dot_float3(a, b) / db
  LET cx AS Float = ratio * b.x
  LET cy AS Float = ratio * b.y
  LET cz AS Float = ratio * b.z
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_project_float4(a AS Float4, b AS Float4) AS Float4
  LET db AS Float = __vector_dot_float4(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = __vector_dot_float4(a, b) / db
  LET cx AS Float = ratio * b.x
  LET cy AS Float = ratio * b.y
  LET cz AS Float = ratio * b.z
  LET cw AS Float = ratio * b.w
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_project_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed2
  LET db AS Fixed = __vector_dot_fixed2(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Fixed = __vector_dot_fixed2(a, b) / db
  LET cx AS Fixed = ratio * b.x
  LET cy AS Fixed = ratio * b.y
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_project_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed3
  LET db AS Fixed = __vector_dot_fixed3(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Fixed = __vector_dot_fixed3(a, b) / db
  LET cx AS Fixed = ratio * b.x
  LET cy AS Fixed = ratio * b.y
  LET cz AS Fixed = ratio * b.z
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_project_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed4
  LET db AS Fixed = __vector_dot_fixed4(b, b)
  IF db = 0.0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Fixed = __vector_dot_fixed4(a, b) / db
  LET cx AS Fixed = ratio * b.x
  LET cy AS Fixed = ratio * b.y
  LET cz AS Fixed = ratio * b.z
  LET cw AS Fixed = ratio * b.w
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_project_integer2(a AS Integer2, b AS Integer2) AS Integer2
  LET db AS Integer = __vector_dot_integer2(b, b)
  IF db = 0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = toFloat(__vector_dot_integer2(a, b)) / toFloat(db)
  LET cx AS Integer = math::round(ratio * toFloat(b.x))
  LET cy AS Integer = math::round(ratio * toFloat(b.y))
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_project_integer3(a AS Integer3, b AS Integer3) AS Integer3
  LET db AS Integer = __vector_dot_integer3(b, b)
  IF db = 0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = toFloat(__vector_dot_integer3(a, b)) / toFloat(db)
  LET cx AS Integer = math::round(ratio * toFloat(b.x))
  LET cy AS Integer = math::round(ratio * toFloat(b.y))
  LET cz AS Integer = math::round(ratio * toFloat(b.z))
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_project_integer4(a AS Integer4, b AS Integer4) AS Integer4
  LET db AS Integer = __vector_dot_integer4(b, b)
  IF db = 0 THEN
    FAIL error(77050002, "vector::project onto a zero-length vector")
  END IF
  LET ratio AS Float = toFloat(__vector_dot_integer4(a, b)) / toFloat(db)
  LET cx AS Integer = math::round(ratio * toFloat(b.x))
  LET cy AS Integer = math::round(ratio * toFloat(b.y))
  LET cz AS Integer = math::round(ratio * toFloat(b.z))
  LET cw AS Integer = math::round(ratio * toFloat(b.w))
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_project_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::project has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "project",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations(
            "project",
            super::Shape::BinaryVector,
            &["ErrInvalidArgument"],
            body,
            &[
                "The vector to project.",
                "The vector to project onto. Must not be zero-length: there is no direction to project onto.",
            ],
        ),
    });
}
