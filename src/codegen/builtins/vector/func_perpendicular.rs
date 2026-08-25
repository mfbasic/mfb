//! `vector::perpendicular` — descriptor entry + the per-type `__vector_perpendicular_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Left perpendicular of a 2D vector"#;

const DESC: &str = r#"`vector::perpendicular` returns `(-v.y, v.x)`, the *left* perpendicular of a 2D
vector: `v` rotated a quarter turn counterclockwise about the origin. The result
is orthogonal to `v` — their dot product is `(-v.y)*v.x + v.x*v.y`, identically
zero — and has exactly the same magnitude, because the two components are merely
swapped and one is negated.

This function is **2D only**. There are just three overloads, one per element
type, and there is no `Float3` or `Float4` form: in three or more dimensions a
single vector does not determine a unique perpendicular, so the operation is not
well defined. Passing a 3D or 4D vector is a compile-time error, not a runtime
one. For the higher-dimensional analogue use `vector::cross`, which takes the
`N - 1` operands needed to pin down a unique orthogonal direction.

The 2D unary form of `vector::cross` computes the same value. The two are
nevertheless **separate functions with separate implementations** in the companion
source — `__vector_perpendicular_float2` and `__vector_cross_float2` — rather than
one delegating to the other; the call dispatches to whichever name you wrote.
Prefer `vector::perpendicular` when the intent is a quarter turn and
`vector::cross` when the intent is the generalized product.

Because the operation is a swap and a single negation, it does no multiplication,
division, or rounding, and is exact on every element type. It is not, however,
completely error-free: the negation `0 - v.y` is checked arithmetic on the
`Fixed` and `Integer` overloads, so a `y` component equal to the minimum
representable value of its type has no representable negation and fails with
`ErrOverflow`. The `Float` overload negates in IEEE arithmetic, where the
negation of any finite value is finite, so it never fails.

Applying `perpendicular` four times returns the original vector, and applying it
twice returns `-v`. Two applications are therefore a cheap exact negation, and
`perpendicular(perpendicular(perpendicular(v)))` is the *right* perpendicular
`(v.y, -v.x)`, which the package does not provide directly.

`vector::perpendicular` accepts only the three **2D** vector record types —
`Float2`, `Fixed2`, and `Integer2`. The overload is selected at compile time from
the exact record type of the single argument; a 3D or 4D vector, a non-vector
argument, or any arity other than one is rejected by the syntax check with the
message that a 2D vector was expected. The return type is always the argument's
own type."#;

const EX: &str = r#"The perpendicular of the `+x` axis is the `+y` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::perpendicular(vector::Float2[1.0, 0.0])))
END SUB
```

Applying it twice negates the vector exactly:

```
IMPORT vector
IMPORT io

SUB main()
  LET back AS vector::Integer2 = vector::perpendicular(vector::perpendicular(vector::Integer2[3, 4]))
  io::print(toString(back))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_perpendicular_float2(v AS Float2) AS Float2
  RETURN Float2[0.0 - v.y, v.x]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_perpendicular_fixed2(v AS Fixed2) AS Fixed2
  RETURN Fixed2[0.0 - v.y, v.x]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_perpendicular_integer2(v AS Integer2) AS Integer2
  RETURN Integer2[0 - v.y, v.x]
END FUNC"#;

/// The `__vector_perpendicular_<type>` body for one applicable vector type.
fn body(ty: &str) -> &'static str {
    match ty {
        "Float2" => BODY_FLOAT2,
        "Fixed2" => BODY_FIXED2,
        "Integer2" => BODY_INTEGER2,
        other => unreachable!("vector::perpendicular has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "perpendicular",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("a 2D vector (Float2, Fixed2, Integer2)"),
        internal_only: false,
        implementations: super::implementations(
            "perpendicular",
            super::Shape::Perpendicular,
            &[],
            body,
        ),
    });
}
