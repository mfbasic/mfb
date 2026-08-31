//! `vector::scale` — descriptor entry + the per-type `__vector_scale_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Component-wise (Hadamard) product of two vectors"#;

const DESC: &str = r#"`vector::scale` returns the component-wise product of two vectors —
`(a.x*b.x, a.y*b.y, ...)`, taking as many terms as the dimension, evaluated in
declared field order. This is the Hadamard product, also called the elementwise
product. Neither argument is modified; a fresh record is returned.

Despite its name, `scale` is **not** multiplication by a scalar: this package
provides no vector-times-scalar function, and both arguments must be full vectors
of the same type. To multiply a whole vector by one number, build a vector whose
components are all that number and pass it as `b` — for a uniform factor of `3`,
`vector::scale(v, vector::Float3[3.0, 3.0, 3.0])`. The usual application of the
general form is non-uniform axis scaling, where each axis is stretched by its own
factor.

It is also not the dot product: `vector::scale` returns a *vector* of the pairwise
products, whereas `vector::dot` sums those same products into a *scalar*. The two
are related by `dot(a, b) = scale(a, b).x + scale(a, b).y + ...`, but they have
different return types and the compiler will not confuse them.

The implementation is multiplication only — no addition beyond that, no division,
no square root, no trigonometry — so it performs **no rounding** on any element
type. The `Integer` overloads are exact checked integer arithmetic and the `Fixed`
overloads are exact within the Q32.32 grid, putting `scale` in the small exact
group alongside `dot`, `cross`, `reflect`, and `perpendicular`. Multiplication is
still ordinary checked arithmetic, however, so a product that leaves the range of
the element type fails with `ErrOverflow` rather than wrapping.

`vector::scale` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and — importantly — no overload takes a bare
scalar as its second argument. The return type is always the first argument's own
type."#;

const EX: &str = r#"The component-wise product of two 2D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::scale(vector::Float2[2.0, 3.0], vector::Float2[4.0, 5.0])))
END SUB
```

Uniform scaling, expressed by repeating the factor in every component:

```
IMPORT vector
IMPORT io

SUB main()
  LET tripled AS vector::Float3 = vector::scale(vector::Float3[1.0, 2.0, 3.0], vector::Float3[3.0, 3.0, 3.0])
  io::print(toString(tripled))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_scale_float2(a AS Float2, b AS Float2) AS Float2
  LET cx AS Float = a.x * b.x
  LET cy AS Float = a.y * b.y
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_scale_float3(a AS Float3, b AS Float3) AS Float3
  LET cx AS Float = a.x * b.x
  LET cy AS Float = a.y * b.y
  LET cz AS Float = a.z * b.z
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_scale_float4(a AS Float4, b AS Float4) AS Float4
  LET cx AS Float = a.x * b.x
  LET cy AS Float = a.y * b.y
  LET cz AS Float = a.z * b.z
  LET cw AS Float = a.w * b.w
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_scale_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed2
  LET cx AS Fixed = a.x * b.x
  LET cy AS Fixed = a.y * b.y
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_scale_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed3
  LET cx AS Fixed = a.x * b.x
  LET cy AS Fixed = a.y * b.y
  LET cz AS Fixed = a.z * b.z
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_scale_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed4
  LET cx AS Fixed = a.x * b.x
  LET cy AS Fixed = a.y * b.y
  LET cz AS Fixed = a.z * b.z
  LET cw AS Fixed = a.w * b.w
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_scale_integer2(a AS Integer2, b AS Integer2) AS Integer2
  LET cx AS Integer = a.x * b.x
  LET cy AS Integer = a.y * b.y
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_scale_integer3(a AS Integer3, b AS Integer3) AS Integer3
  LET cx AS Integer = a.x * b.x
  LET cy AS Integer = a.y * b.y
  LET cz AS Integer = a.z * b.z
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_scale_integer4(a AS Integer4, b AS Integer4) AS Integer4
  LET cx AS Integer = a.x * b.x
  LET cy AS Integer = a.y * b.y
  LET cz AS Integer = a.z * b.z
  LET cw AS Integer = a.w * b.w
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_scale_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::scale has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "scale",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations("scale", super::Shape::BinaryVector, &[], body, &[
            "The vector to scale.",
            "The per-component scale factors, as a vector of the same type. Multiplication is component-wise, not a dot product.",
        ]),
    });
}
