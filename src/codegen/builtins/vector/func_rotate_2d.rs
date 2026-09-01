//! `vector::rotate_2d` — descriptor entry + the per-type `__vector_rotate_2d_*`
//! MFBASIC overload bodies (`Body::mfb`, one per applicable vector type; the
//! overloads are built by the shared `super::implementations` shape machinery).
//! Bodies byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryFunction, RegistryPackage};

const INTRO: &str = r#"Rotate a 2D vector counterclockwise by an angle in radians"#;

const DESC: &str = r#"`vector::rotate_2d` rotates `v` about the origin by `angle` **radians**,
counterclockwise, applying the standard 2D rotation matrix:
`(v.x*cos - v.y*sin, v.x*sin + v.y*cos)`. The sine and cosine are each computed
once and reused for both output components. A positive `angle` turns from the `+x`
axis toward the `+y` axis; a negative `angle` turns the other way. `angle` is
unbounded — it is passed straight to the trigonometric kernels with no range
reduction of its own — so multiple full turns are accepted and behave as the
equivalent angle.

This function is **2D only**. There are just three overloads, one per element
type, and there is no 3D or 4D form: rotation in higher dimensions needs an axis
or a plane, which a single scalar angle cannot specify. Passing a 3D or 4D vector
is a compile-time error.

`angle` is a `Float` for **every** overload, including the `vector::Fixed2` and `vector::Integer2`
ones — it is not the vector's element type, in contrast to
`vector::clamp_length`, whose scalar does follow the element type. The `vector::Float2`
overload uses the in-tree `Float` `math::sin` and `math::cos` directly. The
`vector::Fixed2` and `vector::Integer2` overloads convert `angle` with `toFixed` first and then use
the deterministic Q32.32 `sin` and `cos`, so their results are bit-identical on
every target; that conversion is also a range check, and an `angle` too large to
represent as a `Fixed` fails with `ErrOverflow`.

The `vector::Integer2` overload is the coarsest. It widens both components to `Fixed`,
applies the rotation in Q32.32, and rounds each result back with `math::round`,
half away from zero. Because a rotation generally maps lattice points off the
lattice, the result is snapped to the nearest integer coordinates and the rotation
is therefore not exactly invertible: rotating by an angle and then by its negative
need not return the original vector. Only the multiples of a quarter turn are
exact on `vector::Integer2`, and even those depend on the `Fixed` sine and cosine landing
exactly on `0` and `1`. For an exact quarter turn counterclockwise, prefer
`vector::perpendicular`, which is a pure swap and negation with no trigonometry at
all.

Rotation preserves magnitude on the `vector::Float2` overload up to double-precision
rounding, and approximately on the other two.

`vector::rotate_2d` accepts only the three **2D** vector record types — `vector::Float2`,
`vector::Fixed2`, and `vector::Integer2` — and its second argument must be a `Float` for all
three, with no implicit numeric promotion from `Integer`. A 3D or 4D first
argument, a non-`Float` second argument, or any arity other than two is rejected
by the syntax check with the message that a 2D vector and a `Float` angle were
expected. The return type is always the first argument's own type."#;

const EX: &str = r#"Rotate the `+x` axis by a quarter turn to reach the `+y` axis:

```
IMPORT vector
IMPORT io
IMPORT math

SUB main()
  io::print(toString(vector::rotate_2d(vector::Float2[1.0, 0.0], math::pi2)))
END SUB
```

Rotate clockwise by negating the angle:

```
IMPORT vector
IMPORT io
IMPORT math

SUB main()
  LET cw AS vector::Float2 = vector::rotate_2d(vector::Float2[1.0, 0.0], 0.0 - math::pi2)
  io::print(toString(cw))
END SUB
```"#;

#[rustfmt::skip]
const BODY_FLOAT2: &str =
r#"FUNC __vector_rotate_2d_float2(v AS Float2, ang AS Float) AS Float2
  LET c AS Float = math::cos(ang)
  LET s AS Float = math::sin(ang)
  LET rx AS Float = v.x * c - v.y * s
  LET ry AS Float = v.x * s + v.y * c
  RETURN Float2[rx, ry]
END FUNC"#;
#[rustfmt::skip]
const BODY_FIXED2: &str =
r#"FUNC __vector_rotate_2d_fixed2(v AS Fixed2, ang AS Float) AS Fixed2
  LET af AS Fixed = toFixed(ang)
  LET c AS Fixed = math::cos(af)
  LET s AS Fixed = math::sin(af)
  LET rx AS Fixed = v.x * c - v.y * s
  LET ry AS Fixed = v.x * s + v.y * c
  RETURN Fixed2[rx, ry]
END FUNC"#;
#[rustfmt::skip]
const BODY_INTEGER2: &str =
r#"FUNC __vector_rotate_2d_integer2(v AS Integer2, ang AS Float) AS Integer2
  LET af AS Fixed = toFixed(ang)
  LET c AS Fixed = math::cos(af)
  LET s AS Fixed = math::sin(af)
  LET xf AS Fixed = toFixed(v.x)
  LET yf AS Fixed = toFixed(v.y)
  LET rx AS Integer = math::round(xf * c - yf * s)
  LET ry AS Integer = math::round(xf * s + yf * c)
  RETURN Integer2[rx, ry]
END FUNC"#;

/// The `__vector_rotate_2d_<type>` body for one applicable vector type.
fn body(ty: &str) -> &'static str {
    match ty {
        "Float2" => BODY_FLOAT2,
        "Fixed2" => BODY_FIXED2,
        "Integer2" => BODY_INTEGER2,
        other => unreachable!("vector::rotate_2d has no {other} overload"),
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "rotate_2d",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("a 2D vector and a Float angle"),
        internal_only: false,
        implementations: super::implementations("rotate_2d", super::Shape::Rotate2d, &[], body, &[
            "The 2D vector to rotate.",
            "How far to rotate, in radians, counter-clockwise. Negative values rotate clockwise.",
        ]),
    });
}
