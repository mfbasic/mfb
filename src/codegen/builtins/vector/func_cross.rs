//! `vector::cross` — descriptor entry + the per-type `__vector_cross_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Generalized (n-1)-ary cross product"#;

const DESC: &str = r#"`vector::cross` returns a vector orthogonal to all of its operands. It is the
*generalized* cross product, which in an N-dimensional space takes `N - 1`
operands, so the arity of this call is fixed by the dimension of the vector type:
one operand in 2D, two in 3D, three in 4D. Passing the wrong number of operands
for the dimension — a single `vector::Float3`, or two `vector::Float4` values — is a compile-time
error, not a runtime one.

In 2D the unary form returns the *left perpendicular* `(-v.y, v.x)`, which is `v`
rotated a quarter turn counterclockwise. In 3D it is the familiar binary product
`a x b`, whose components are `(a.y*b.z - a.z*b.y, a.z*b.x - a.x*b.z, a.x*b.y - a.y*b.x)`;
it follows the right-hand rule, so `cross(right, up)` yields `forward`. In 4D it
is the ternary product built from the six 2x2 minors of `b` and `c` expanded
against `a`, in the cofactor pattern
`(a.y*mZW - a.z*mYW + a.w*mYZ, a.z*mXW - a.x*mZW - a.w*mXZ, a.x*mYW - a.y*mXW + a.w*mXY, a.y*mXZ - a.x*mYZ - a.z*mXY)`.
Note the sign convention this particular expansion implies: `cross` of the `x`,
`y`, and `z` basis vectors yields the **negated** `w` axis, `(0, 0, 0, -1)`, not
`(0, 0, 0, 1)`.

Every form is built from multiplications and subtractions only — there is no
division, no square root, and no trigonometry anywhere in any overload. As a
result `cross` performs **no rounding** on any element type: the `Integer`
overloads are exact integer arithmetic and the `Fixed` overloads are exact within
the Q32.32 grid, in contrast to `normalize`, `project`, and the interpolation
functions, which all round on `Integer`. `cross` is also the only geometry
function here that never raises `ErrInvalidArgument`: it has no degenerate input
to reject, and the cross product of parallel operands is simply the zero vector.

The unary 2D form gives the same result as `vector::perpendicular`. Use whichever
name reads better where you are: `cross` when the surrounding code is doing
cross products across dimensions, `perpendicular` when the point is the quarter
turn itself.

`vector::cross` is generic over the nine built-in vector record types, and is the
only member of this package whose accepted arity varies: `1` for a 2D type, `2`
for a 3D type, `3` for a 4D type. The declared arity span is therefore `1` through
`3`, with the exact requirement enforced against the first argument's dimension
during overload resolution. Every operand must be the *same* one of the nine
types; there is no mixed-element-type and no cross-dimension overload."#;

const EX: &str = r#"The 3D cross product of the `x` and `y` basis vectors is the `z` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::cross(vector::Float3[1.0, 0.0, 0.0], vector::Float3[0.0, 1.0, 0.0])))
END SUB
```

The unary 2D form is a quarter turn counterclockwise:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::cross(vector::Float2[1.0, 0.0])))
END SUB
```

The ternary 4D form, orthogonal to all three basis operands:

```
IMPORT vector
IMPORT io

SUB main()
  LET n AS vector::Float4 = vector::cross(vector::Float4[1.0, 0.0, 0.0, 0.0], vector::Float4[0.0, 1.0, 0.0, 0.0], vector::Float4[0.0, 0.0, 1.0, 0.0])
  io::print(toString(n))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_cross_float2(v AS Float2) AS Float2
  RETURN Float2[0.0 - v.y, v.x]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_cross_float3(a AS Float3, b AS Float3) AS Float3
  LET cx AS Float = a.y * b.z - a.z * b.y
  LET cy AS Float = a.z * b.x - a.x * b.z
  LET cz AS Float = a.x * b.y - a.y * b.x
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_cross_float4(a AS Float4, b AS Float4, c AS Float4) AS Float4
  LET mZW AS Float = b.z * c.w - b.w * c.z
  LET mYW AS Float = b.y * c.w - b.w * c.y
  LET mYZ AS Float = b.y * c.z - b.z * c.y
  LET mXW AS Float = b.x * c.w - b.w * c.x
  LET mXZ AS Float = b.x * c.z - b.z * c.x
  LET mXY AS Float = b.x * c.y - b.y * c.x
  LET rx AS Float = a.y * mZW - a.z * mYW + a.w * mYZ
  LET ry AS Float = a.z * mXW - a.x * mZW - a.w * mXZ
  LET rz AS Float = a.x * mYW - a.y * mXW + a.w * mXY
  LET rw AS Float = a.y * mXZ - a.x * mYZ - a.z * mXY
  RETURN Float4[rx, ry, rz, rw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_cross_fixed2(v AS Fixed2) AS Fixed2
  RETURN Fixed2[0.0 - v.y, v.x]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_cross_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed3
  LET cx AS Fixed = a.y * b.z - a.z * b.y
  LET cy AS Fixed = a.z * b.x - a.x * b.z
  LET cz AS Fixed = a.x * b.y - a.y * b.x
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_cross_fixed4(a AS Fixed4, b AS Fixed4, c AS Fixed4) AS Fixed4
  LET mZW AS Fixed = b.z * c.w - b.w * c.z
  LET mYW AS Fixed = b.y * c.w - b.w * c.y
  LET mYZ AS Fixed = b.y * c.z - b.z * c.y
  LET mXW AS Fixed = b.x * c.w - b.w * c.x
  LET mXZ AS Fixed = b.x * c.z - b.z * c.x
  LET mXY AS Fixed = b.x * c.y - b.y * c.x
  LET rx AS Fixed = a.y * mZW - a.z * mYW + a.w * mYZ
  LET ry AS Fixed = a.z * mXW - a.x * mZW - a.w * mXZ
  LET rz AS Fixed = a.x * mYW - a.y * mXW + a.w * mXY
  LET rw AS Fixed = a.y * mXZ - a.x * mYZ - a.z * mXY
  RETURN Fixed4[rx, ry, rz, rw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_cross_integer2(v AS Integer2) AS Integer2
  RETURN Integer2[0 - v.y, v.x]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_cross_integer3(a AS Integer3, b AS Integer3) AS Integer3
  LET cx AS Integer = a.y * b.z - a.z * b.y
  LET cy AS Integer = a.z * b.x - a.x * b.z
  LET cz AS Integer = a.x * b.y - a.y * b.x
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_cross_integer4(a AS Integer4, b AS Integer4, c AS Integer4) AS Integer4
  LET mZW AS Integer = b.z * c.w - b.w * c.z
  LET mYW AS Integer = b.y * c.w - b.w * c.y
  LET mYZ AS Integer = b.y * c.z - b.z * c.y
  LET mXW AS Integer = b.x * c.w - b.w * c.x
  LET mXZ AS Integer = b.x * c.z - b.z * c.x
  LET mXY AS Integer = b.x * c.y - b.y * c.x
  LET rx AS Integer = a.y * mZW - a.z * mYW + a.w * mYZ
  LET ry AS Integer = a.z * mXW - a.x * mZW - a.w * mXZ
  LET rz AS Integer = a.x * mYW - a.y * mXW + a.w * mXY
  LET rw AS Integer = a.y * mXZ - a.x * mYZ - a.z * mXY
  RETURN Integer4[rx, ry, rz, rw]
END FUNC"#;

/// The `__vector_cross_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::cross has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "cross",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("one T2, two T3, or three T4 vectors of the same type"),
        internal_only: false,
        implementations: super::implementations("cross", super::Shape::Cross, &[], body, &[
            "The first vector. In 2D this is the only argument and the result is the perpendicular; in 3D and 4D it is the left operand.",
            "The second vector (3D and 4D only).",
            "The third vector (4D only).",
        ]),
    });
}
