//! `vector::dot` — descriptor entry + the per-type `__vector_dot_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Dot (inner) product of two vectors"#;

const DESC: &str = r#"`vector::dot` returns the sum of the products of corresponding components:
`a.x*b.x + a.y*b.y + a.z*b.z + a.w*b.w`, taking as many terms as the dimension.
The products are formed and accumulated strictly left to right in declared field
order, which is what makes the result reproducible bit for bit across targets.
The dot product is symmetric — `dot(a, b)` equals `dot(b, a)` — and is a scalar,
so the return type is the vector type's element type rather than a vector.

Geometrically the dot product equals `length(a) * length(b) * cos(angle(a, b))`,
which makes its **sign** the useful part in most code: positive when the two
vectors point broadly the same way (their angle is under a quarter turn), zero
when they are exactly orthogonal, and negative when they point broadly opposite
ways. `dot(v, v)` is the squared length of `v`, which is why several other
functions in this package — `project`, `reject`, `angle`, and the `Integer`
`normalize` — use it to test for a zero-length vector without paying for a square
root.

`dot` is multiplication and addition only: no division, no square
root, and no trigonometry. It therefore performs **no rounding** on any element
type. The `Integer` overloads are exact checked integer arithmetic, so
`vector::dot` is one of the few members of this package (with `cross` and `scale`)
whose `Integer` results carry no approximation at all. The `Fixed` overloads are
exact within the Q32.32 grid, and the `Float` overloads are ordinary IEEE
double arithmetic.

Because the terms are ordinary checked arithmetic, `dot` can overflow. Squaring
a large coordinate is the common way to hit this: `dot(v, v)` on a `vector::Integer3`
whose components approach the square root of the `Integer` maximum will exceed
the range and fail with `ErrOverflow`. There are no other failure modes — `dot`
never rejects an input, and the zero vector is an entirely ordinary argument
returning zero.

`vector::dot` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is the element type of that vector type, not the vector
type itself."#;

const EX: &str = r#"The dot product of two 3D vectors:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::dot(vector::Float3[1.0, 2.0, 3.0], vector::Float3[4.0, 5.0, 6.0])))
END SUB
```

Using the sign to test whether two directions broadly agree:

```
IMPORT vector
IMPORT io

SUB main()
  LET facing AS Float = vector::dot(vector::Float2[1.0, 0.0], vector::Float2[0.0 - 1.0, 0.0])
  IF facing < 0.0 THEN
    io::print("opposite")
  END IF
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_dot_float2(a AS Float2, b AS Float2) AS Float
  RETURN a.x * b.x + a.y * b.y
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_dot_float3(a AS Float3, b AS Float3) AS Float
  RETURN a.x * b.x + a.y * b.y + a.z * b.z
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_dot_float4(a AS Float4, b AS Float4) AS Float
  RETURN a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_dot_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed
  RETURN a.x * b.x + a.y * b.y
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_dot_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed
  RETURN a.x * b.x + a.y * b.y + a.z * b.z
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_dot_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed
  RETURN a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_dot_integer2(a AS Integer2, b AS Integer2) AS Integer
  RETURN a.x * b.x + a.y * b.y
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_dot_integer3(a AS Integer3, b AS Integer3) AS Integer
  RETURN a.x * b.x + a.y * b.y + a.z * b.z
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_dot_integer4(a AS Integer4, b AS Integer4) AS Integer
  RETURN a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
END FUNC"#;

/// The `__vector_dot_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::dot has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "dot",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations(
            "dot",
            super::Shape::BinaryScalar,
            &[],
            body,
            &[
                "The first vector.",
                "The second vector. `dot` is symmetric, so the order does not matter.",
            ],
        ),
    });
}
