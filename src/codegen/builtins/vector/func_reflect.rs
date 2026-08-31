//! `vector::reflect` — descriptor entry + the per-type `__vector_reflect_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Reflect a vector about a plane through the origin with the given normal"#;

const DESC: &str = r#"`vector::reflect` returns `v - 2 * dot(v, n) * n`, the mirror image of `v` across
the hyperplane through the origin whose normal is `n`. The scalar `2 * dot(v, n)`
is formed once and then multiplies each component of `n` in declared field order,
with the product subtracted from the corresponding component of `v`. This is the
classic bounce formula: the component of `v` along `n` is negated while the
component within the plane is left untouched.

**`n` is used exactly as given and is never normalized.** The formula is only a
true reflection when `n` is a unit vector; if `n` has length `k`, the term
`2 * dot(v, n) * n` scales by `k^2` and the result is not a mirror image but a
skewed vector whose magnitude generally differs from `v`'s. Callers are
responsible for passing a unit normal — typically the output of
`vector::normalize` — and this function will not do it for them. In exchange,
`reflect` never rejects an input: unlike `vector::project` and `vector::reject`,
it has no division and therefore no zero-vector guard, so a zero `n` is accepted
and simply returns `v` unchanged.

Because `reflect` is multiplication and subtraction only — no division,
no square root, no trigonometry — it performs **no rounding** on any element type.
The `Integer` overloads are exact checked integer arithmetic and the `Fixed`
overloads are exact within the Q32.32 grid. This puts `reflect` in the small group
of exact members of this package alongside `dot`, `cross`, `scale`, and
`perpendicular`, and means that reflecting an `Integer` vector about an `Integer`
unit axis such as `(0, 1)` is exact.

Reflection is its own inverse for a unit normal: applying `reflect` twice with the
same `n` returns the original vector. It also preserves magnitude for a unit
normal, and reverses the sign of `dot(v, n)` while leaving every in-plane
component fixed.

`vector::reflect` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type."#;

const EX: &str = r#"Bounce a downward-moving vector off a floor whose normal is the `+y` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::reflect(vector::Float2[1.0, 0.0 - 1.0], vector::Float2[0.0, 1.0])))
END SUB
```

Normalize the surface normal first when it is not already a unit vector:

```
IMPORT vector
IMPORT io

SUB main()
  LET n AS vector::Float3 = vector::normalize(vector::Float3[0.0, 3.0, 4.0])
  io::print(toString(vector::reflect(vector::Float3[1.0, 0.0 - 1.0, 0.0], n)))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_reflect_float2(v AS Float2, n AS Float2) AS Float2
  LET d AS Float = __vector_dot_float2(v, n)
  LET k AS Float = 2.0 * d
  LET cx AS Float = v.x - k * n.x
  LET cy AS Float = v.y - k * n.y
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_reflect_float3(v AS Float3, n AS Float3) AS Float3
  LET d AS Float = __vector_dot_float3(v, n)
  LET k AS Float = 2.0 * d
  LET cx AS Float = v.x - k * n.x
  LET cy AS Float = v.y - k * n.y
  LET cz AS Float = v.z - k * n.z
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_reflect_float4(v AS Float4, n AS Float4) AS Float4
  LET d AS Float = __vector_dot_float4(v, n)
  LET k AS Float = 2.0 * d
  LET cx AS Float = v.x - k * n.x
  LET cy AS Float = v.y - k * n.y
  LET cz AS Float = v.z - k * n.z
  LET cw AS Float = v.w - k * n.w
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_reflect_fixed2(v AS Fixed2, n AS Fixed2) AS Fixed2
  LET d AS Fixed = __vector_dot_fixed2(v, n)
  LET k AS Fixed = toFixed(2.0) * d
  LET cx AS Fixed = v.x - k * n.x
  LET cy AS Fixed = v.y - k * n.y
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_reflect_fixed3(v AS Fixed3, n AS Fixed3) AS Fixed3
  LET d AS Fixed = __vector_dot_fixed3(v, n)
  LET k AS Fixed = toFixed(2.0) * d
  LET cx AS Fixed = v.x - k * n.x
  LET cy AS Fixed = v.y - k * n.y
  LET cz AS Fixed = v.z - k * n.z
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_reflect_fixed4(v AS Fixed4, n AS Fixed4) AS Fixed4
  LET d AS Fixed = __vector_dot_fixed4(v, n)
  LET k AS Fixed = toFixed(2.0) * d
  LET cx AS Fixed = v.x - k * n.x
  LET cy AS Fixed = v.y - k * n.y
  LET cz AS Fixed = v.z - k * n.z
  LET cw AS Fixed = v.w - k * n.w
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_reflect_integer2(v AS Integer2, n AS Integer2) AS Integer2
  LET d AS Integer = __vector_dot_integer2(v, n)
  LET k AS Integer = 2 * d
  LET cx AS Integer = v.x - k * n.x
  LET cy AS Integer = v.y - k * n.y
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_reflect_integer3(v AS Integer3, n AS Integer3) AS Integer3
  LET d AS Integer = __vector_dot_integer3(v, n)
  LET k AS Integer = 2 * d
  LET cx AS Integer = v.x - k * n.x
  LET cy AS Integer = v.y - k * n.y
  LET cz AS Integer = v.z - k * n.z
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_reflect_integer4(v AS Integer4, n AS Integer4) AS Integer4
  LET d AS Integer = __vector_dot_integer4(v, n)
  LET k AS Integer = 2 * d
  LET cx AS Integer = v.x - k * n.x
  LET cy AS Integer = v.y - k * n.y
  LET cz AS Integer = v.z - k * n.z
  LET cw AS Integer = v.w - k * n.w
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_reflect_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::reflect has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "reflect",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations("reflect", super::Shape::BinaryVector, &[], body, &[
            "The incoming vector to reflect.",
            "The surface normal to reflect across. Give it unit length — `reflect` does not normalize it for you, and a longer normal scales the result.",
        ]),
    });
}
