//! `vector::min` — descriptor entry + the per-type `__vector_min_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Component-wise minimum of two vectors"#;

const DESC: &str = r#"`vector::min` returns a new vector whose every component is the smaller of the two
corresponding components, each computed by the scalar `math::min` in declared
field order. The result is assembled into a fresh record; neither argument is
modified.

The comparison is made **per component and independently**, so the returned
vector is generally not equal to either input: `min(Float2[2.0, 3.0], Float2[4.0, 1.0])`
is `(2.0, 1.0)`, which is neither operand. This is the corner-wise lower bound of
the two vectors, not a selection of the shorter one — `vector::min` does not
compare magnitudes and is not related to `vector::length`. Paired with
`vector::max` it is the standard way to build an axis-aligned bounding box: `min`
gives the low corner and `max` the high corner.

The operation is a comparison and a select on every element type — it does no
arithmetic at all, so it cannot overflow, performs no rounding, and never fails.
This makes `vector::min` one of only two functions in this package (with
`vector::max`) that raise no errors whatsoever, on any overload. `Float`
comparisons use the hardware minimum instruction; `Fixed`, `Integer`, and `Money`
comparisons are a signed 64-bit compare and select over the underlying
representation.

`vector::min` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type."#;

const EX: &str = r#"The component-wise minimum of two 2D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::min(vector::Float2[2.0, 3.0], vector::Float2[4.0, 1.0])))
END SUB
```

The low corner of a bounding box around two points:

```
IMPORT vector
IMPORT io

SUB main()
  LET lo AS vector::Integer3 = vector::min(vector::Integer3[1, 7, 3], vector::Integer3[4, 2, 9])
  io::print(toString(lo))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_min_float2(a AS Float2, b AS Float2) AS Float2
  LET cx AS Float = math::min(a.x, b.x)
  LET cy AS Float = math::min(a.y, b.y)
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_min_float3(a AS Float3, b AS Float3) AS Float3
  LET cx AS Float = math::min(a.x, b.x)
  LET cy AS Float = math::min(a.y, b.y)
  LET cz AS Float = math::min(a.z, b.z)
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_min_float4(a AS Float4, b AS Float4) AS Float4
  LET cx AS Float = math::min(a.x, b.x)
  LET cy AS Float = math::min(a.y, b.y)
  LET cz AS Float = math::min(a.z, b.z)
  LET cw AS Float = math::min(a.w, b.w)
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_min_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed2
  LET cx AS Fixed = math::min(a.x, b.x)
  LET cy AS Fixed = math::min(a.y, b.y)
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_min_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed3
  LET cx AS Fixed = math::min(a.x, b.x)
  LET cy AS Fixed = math::min(a.y, b.y)
  LET cz AS Fixed = math::min(a.z, b.z)
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_min_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed4
  LET cx AS Fixed = math::min(a.x, b.x)
  LET cy AS Fixed = math::min(a.y, b.y)
  LET cz AS Fixed = math::min(a.z, b.z)
  LET cw AS Fixed = math::min(a.w, b.w)
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_min_integer2(a AS Integer2, b AS Integer2) AS Integer2
  LET cx AS Integer = math::min(a.x, b.x)
  LET cy AS Integer = math::min(a.y, b.y)
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_min_integer3(a AS Integer3, b AS Integer3) AS Integer3
  LET cx AS Integer = math::min(a.x, b.x)
  LET cy AS Integer = math::min(a.y, b.y)
  LET cz AS Integer = math::min(a.z, b.z)
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_min_integer4(a AS Integer4, b AS Integer4) AS Integer4
  LET cx AS Integer = math::min(a.x, b.x)
  LET cy AS Integer = math::min(a.y, b.y)
  LET cz AS Integer = math::min(a.z, b.z)
  LET cw AS Integer = math::min(a.w, b.w)
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_min_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::min has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "min",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations("min", super::Shape::BinaryVector, &[], body),
    });
}
