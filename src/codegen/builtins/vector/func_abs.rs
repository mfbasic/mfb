//! `vector::abs` — descriptor entry + the per-type `__vector_abs_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Component-wise absolute value of a vector"#;

const DESC: &str = r#"`vector::abs` returns a new vector of the same type whose every component is the
absolute value of the corresponding component of `v`. Each component is computed
by the scalar `math::abs` of that component, evaluated in declared field order
(`x`, then `y`, then `z`, then `w`), and the results are assembled into a fresh
record. `v` is not modified — like every `vector` type these records copy by
value.

This is a purely component-wise operation with no cross-component interaction:
`abs` reflects the vector into the all-positive orthant, so it is not a
direction-preserving operation and the result generally does not point the same
way as `v`. The magnitude is preserved, however, because negating individual
components does not change the sum of their squares — `vector::length(vector::abs(v))`
always equals `vector::length(v)`.

The three element types differ only in how the scalar absolute value is taken.
The `Float` overloads clear the sign bit with the hardware floating-point
absolute value, which cannot overflow and performs no rounding or domain check,
so the `Float` overloads never fail. The `Fixed` and `Integer` overloads operate
on the underlying signed 64-bit representation, whose negative range extends one
step further than its positive range; negating the minimum representable value
has no positive counterpart and is reported as `ErrOverflow` rather than
silently wrapping. This is exactly the scalar `math::abs` behavior, inherited
per component.

`vector::abs` is generic over the nine built-in vector record types. The overload
is selected at compile time from the exact record type of the single argument;
no implicit conversion or numeric promotion is applied to a vector argument, and
a non-vector argument or any arity other than one is rejected by the syntax
check. The return type is always the argument's own type."#;

const EX: &str = r#"Absolute value of a `Float2`:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::abs(vector::Float2[0.0 - 2.0, 3.0])))
END SUB
```

Absolute value of an `Integer3`:

```
IMPORT vector
IMPORT io

SUB main()
  LET a AS vector::Integer3 = vector::abs(vector::Integer3[0 - 3, 4, 0 - 5])
  io::print(toString(a))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_abs_float2(v AS Float2) AS Float2
  LET cx AS Float = math::abs(v.x)
  LET cy AS Float = math::abs(v.y)
  RETURN Float2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_abs_float3(v AS Float3) AS Float3
  LET cx AS Float = math::abs(v.x)
  LET cy AS Float = math::abs(v.y)
  LET cz AS Float = math::abs(v.z)
  RETURN Float3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_abs_float4(v AS Float4) AS Float4
  LET cx AS Float = math::abs(v.x)
  LET cy AS Float = math::abs(v.y)
  LET cz AS Float = math::abs(v.z)
  LET cw AS Float = math::abs(v.w)
  RETURN Float4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_abs_fixed2(v AS Fixed2) AS Fixed2
  LET cx AS Fixed = math::abs(v.x)
  LET cy AS Fixed = math::abs(v.y)
  RETURN Fixed2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_abs_fixed3(v AS Fixed3) AS Fixed3
  LET cx AS Fixed = math::abs(v.x)
  LET cy AS Fixed = math::abs(v.y)
  LET cz AS Fixed = math::abs(v.z)
  RETURN Fixed3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_abs_fixed4(v AS Fixed4) AS Fixed4
  LET cx AS Fixed = math::abs(v.x)
  LET cy AS Fixed = math::abs(v.y)
  LET cz AS Fixed = math::abs(v.z)
  LET cw AS Fixed = math::abs(v.w)
  RETURN Fixed4[cx, cy, cz, cw]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_abs_integer2(v AS Integer2) AS Integer2
  LET cx AS Integer = math::abs(v.x)
  LET cy AS Integer = math::abs(v.y)
  RETURN Integer2[cx, cy]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_abs_integer3(v AS Integer3) AS Integer3
  LET cx AS Integer = math::abs(v.x)
  LET cy AS Integer = math::abs(v.y)
  LET cz AS Integer = math::abs(v.z)
  RETURN Integer3[cx, cy, cz]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_abs_integer4(v AS Integer4) AS Integer4
  LET cx AS Integer = math::abs(v.x)
  LET cy AS Integer = math::abs(v.y)
  LET cz AS Integer = math::abs(v.z)
  LET cw AS Integer = math::abs(v.w)
  RETURN Integer4[cx, cy, cz, cw]
END FUNC"#;

/// The `__vector_abs_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::abs has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "abs",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("a vector (Float2/3/4, Fixed2/3/4, Integer2/3/4)"),
        internal_only: false,
        implementations: super::implementations("abs", super::Shape::UnaryVector, &[], body),
    });
}
