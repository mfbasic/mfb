//! `vector::length` — descriptor entry + the per-type `__vector_length_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Euclidean length (magnitude) of a vector"#;

const DESC: &str = r#"`vector::length` returns the Euclidean magnitude of `v`: the square root of the
sum of its squared components, `sqrt(x*x + y*y + ...)`, taking as many terms as
the dimension. The squares are accumulated strictly left to right in declared
field order, `x` before `y` before `z` before `w`, which is what makes the result
reproducible bit for bit across targets. The result is always non-negative, and
is zero exactly when every component of `v` is zero.

The return type is the vector type's **element** type, not the vector type: a
`vector::Float4` measures to a `Float`, a `vector::Fixed2` to a `Fixed`, a `vector::Integer3` to an
`Integer`. The zero vector is an entirely ordinary argument here — `length` has no
degenerate input to reject and never raises `ErrInvalidArgument`, in contrast to
`vector::normalize`, which needs a direction and refuses the zero vector.

The `Float` overloads sum in IEEE doubles and take the root with `math::sqrt`.
The `Fixed` overloads work entirely in deterministic Q32.32 arithmetic. The
`Integer` overloads square and sum in exact checked integer arithmetic and then
apply the package's rounding integer square root: it first derives a seed from
an approximate square root of the sum, then corrects it to the exact
`floor` of the true root using only integer comparisons and divisions, and finally
rounds up when the remainder exceeds the floor. The floating-point seed is only a
starting point — the integer correction loops guarantee the exact floor
regardless of how the seed rounded — so the `Integer` result is deterministic and
independent of the host's floating-point behavior.

The rounding rule for the `Integer` overloads is half away from zero, matching
`math::round`. Because `(f + 0.5)^2` is never an integer, no exact tie can ever
occur, so the rule is unambiguous in practice: the result rounds up exactly when
the remainder above the floor exceeds the floor itself. An `Integer` length is
therefore the nearest integer to the true magnitude, not a truncation —
`length(vector::Integer2[3, 4])` is exactly `5`, and `length(vector::Integer2[1, 1])` is `1`.

`vector::length` is generic over the nine built-in vector record types. The
overload is selected at compile time from the exact record type of the single
argument; no implicit conversion or numeric promotion is applied to a vector
argument, and a non-vector argument or any arity other than one is rejected by the
syntax check. The return type is the element type of that vector type, not the
vector type itself."#;

const EX: &str = r#"The length of a 3-4-5 vector:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::length(vector::Float3[3.0, 0.0, 4.0])))
END SUB
```

An `Integer` length rounds to the nearest whole unit:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::length(vector::Integer2[3, 4])))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_length_float2(v AS Float2) AS Float
  RETURN math::sqrt(v.x * v.x + v.y * v.y)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_length_float3(v AS Float3) AS Float
  RETURN math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_length_float4(v AS Float4) AS Float
  RETURN math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_length_fixed2(v AS Fixed2) AS Fixed
  RETURN math::sqrt(v.x * v.x + v.y * v.y)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_length_fixed3(v AS Fixed3) AS Fixed
  RETURN math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z)
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_length_fixed4(v AS Fixed4) AS Fixed
  RETURN math::sqrt(v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_length_integer2(v AS Integer2) AS Integer
  LET s AS Integer = v.x * v.x + v.y * v.y
  RETURN __vector_isqrtRound(s)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_length_integer3(v AS Integer3) AS Integer
  LET s AS Integer = v.x * v.x + v.y * v.y + v.z * v.z
  RETURN __vector_isqrtRound(s)
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_length_integer4(v AS Integer4) AS Integer
  LET s AS Integer = v.x * v.x + v.y * v.y + v.z * v.z + v.w * v.w
  RETURN __vector_isqrtRound(s)
END FUNC"#;

/// The `__vector_length_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::length has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "length",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("a vector (Float2/3/4, Fixed2/3/4, Integer2/3/4)"),
        internal_only: false,
        implementations: super::implementations(
            "length",
            super::Shape::UnaryScalar,
            &[],
            body,
            &["The vector to measure. The zero vector has length zero."],
        ),
    });
}
