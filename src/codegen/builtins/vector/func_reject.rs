//! `vector::reject` — descriptor entry + the per-type `__vector_reject_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Component of one vector orthogonal to another"#;

const DESC: &str = r#"`vector::reject` returns the part of `a` that is perpendicular to `b` — the vector
rejection, the complement of the vector projection. It is implemented directly as
`a - vector::project(a, b)`: it takes the same `project`
helper for the same type and then subtracts its components from `a`'s, in declared
field order.

Because `reject` is defined by that subtraction, the decomposition identity
`project(a, b) + reject(a, b) = a` holds **exactly**, on every element type,
including `Integer`. This is worth stating precisely: the projection itself is
rounded on the `Integer` overloads, so it is only an approximation of the true
parallel component, and consequently the `Integer` rejection is only approximately
orthogonal to `b`. What is exact is the round trip — whatever error the rounding
introduced into the projection is absorbed into the rejection, so the two always
sum back to `a` with no residue.

Delegating to `project` also means `reject` inherits its precondition. **`b` must
not be the zero vector**: the underlying `project` computes `dot(b, b)` and fails
with `ErrInvalidArgument` and the message
`vector::project onto a zero-length vector` when it is zero. Note that the message
names `project`, not `reject`, because the failure is raised inside the delegated
call. `a` is unconstrained — the zero vector rejects to the zero vector. Only the
direction of `b` matters, not its magnitude.

When `a` is already orthogonal to `b` the projection is the zero vector and
`reject` returns `a` unchanged; when `a` is parallel to `b` the projection is all
of `a` and `reject` returns the zero vector. The result is always orthogonal to
`b` on the `Float` and `Fixed` overloads, to within the precision of the element
type.

`vector::reject` is generic over the nine built-in vector record types. Both
arguments must be the *same* one of the nine types: there is no mixed-element-type
and no cross-dimension overload, and no implicit conversion is applied to a vector
argument. The return type is always the first argument's own type."#;

const EX: &str = r#"The part of a diagonal vector that is not along the `+x` axis:

```
IMPORT vector
IMPORT io

SUB main()
  io::print(toString(vector::reject(vector::Float2[2.0, 2.0], vector::Float2[1.0, 0.0])))
END SUB
```

Flatten a movement vector so it slides along a wall instead of passing through
it:

```
IMPORT vector
IMPORT io

SUB main()
  LET wall AS vector::Float3 = vector::normalize(vector::Float3[1.0, 0.0, 0.0])
  LET slide AS vector::Float3 = vector::reject(vector::Float3[1.0, 2.0, 0.0], wall)
  io::print(toString(slide))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_reject_float2(a AS Float2, b AS Float2) AS Float2
  LET p AS Float2 = __vector_project_float2(a, b)
  RETURN Float2[a.x - p.x, a.y - p.y]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT3: &str =
r#"FUNC __vector_reject_float3(a AS Float3, b AS Float3) AS Float3
  LET p AS Float3 = __vector_project_float3(a, b)
  RETURN Float3[a.x - p.x, a.y - p.y, a.z - p.z]
END FUNC"#;
#[rustfmt::skip]
const BODY_FLOAT4: &str =
r#"FUNC __vector_reject_float4(a AS Float4, b AS Float4) AS Float4
  LET p AS Float4 = __vector_project_float4(a, b)
  RETURN Float4[a.x - p.x, a.y - p.y, a.z - p.z, a.w - p.w]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_reject_fixed2(a AS Fixed2, b AS Fixed2) AS Fixed2
  LET p AS Fixed2 = __vector_project_fixed2(a, b)
  RETURN Fixed2[a.x - p.x, a.y - p.y]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED3: &str =
r#"FUNC __vector_reject_fixed3(a AS Fixed3, b AS Fixed3) AS Fixed3
  LET p AS Fixed3 = __vector_project_fixed3(a, b)
  RETURN Fixed3[a.x - p.x, a.y - p.y, a.z - p.z]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED4: &str =
r#"FUNC __vector_reject_fixed4(a AS Fixed4, b AS Fixed4) AS Fixed4
  LET p AS Fixed4 = __vector_project_fixed4(a, b)
  RETURN Fixed4[a.x - p.x, a.y - p.y, a.z - p.z, a.w - p.w]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_reject_integer2(a AS Integer2, b AS Integer2) AS Integer2
  LET p AS Integer2 = __vector_project_integer2(a, b)
  RETURN Integer2[a.x - p.x, a.y - p.y]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER3: &str =
r#"FUNC __vector_reject_integer3(a AS Integer3, b AS Integer3) AS Integer3
  LET p AS Integer3 = __vector_project_integer3(a, b)
  RETURN Integer3[a.x - p.x, a.y - p.y, a.z - p.z]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER4: &str =
r#"FUNC __vector_reject_integer4(a AS Integer4, b AS Integer4) AS Integer4
  LET p AS Integer4 = __vector_project_integer4(a, b)
  RETURN Integer4[a.x - p.x, a.y - p.y, a.z - p.z, a.w - p.w]
END FUNC"#;

/// The `__vector_reject_<type>` body for one applicable vector type.
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
        other => unreachable!("vector::reject has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "reject",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("two vectors of the same type"),
        internal_only: false,
        implementations: super::implementations(
            "reject",
            super::Shape::BinaryVector,
            &["ErrInvalidArgument"],
            body,
            &[
                "The vector to take the rejection of.",
                "The vector to reject from. Must not be zero-length, for the same reason as `project`.",
            ],
        ),
    });
}
