#!/usr/bin/env python3
"""Generate src/builtins/vector_package.mfb (plan-06-vector.md spine, Phases 1-6).

The nine vector value records and every overloaded geometry/utility/2D function
follow strict per-(element-type, dimension) patterns; generating the companion
keeps the ~170 overloads consistent and the canonical left-to-right evaluation
order (plan-06 §4) uniform. The emitted file is the committed source of truth;
re-run this script when the semantics change.

Determinism: every algebraic op is correctly-rounded (FSQRT / IEEE +-*/),
Fixed is Q32.32, Integer uses a deterministic rounding isqrt; the three trig
members route to math::'s deterministic Fixed/Float kernels. Integer results
from a real-valued computation round half away from zero (math::round).
"""

import sys

ELEMENTS = ["Float", "Fixed", "Integer"]
DIMS = [2, 3, 4]
FIELDS = {2: ["x", "y"], 3: ["x", "y", "z"], 4: ["x", "y", "z", "w"]}


def tname(element, dim):
    return f"{element}{dim}"


def impl(func, element, dim):
    return f"__vector_{func}_{element.lower()}{dim}"


def zero(element):
    return "0" if element == "Integer" else "0.0"


def lit(element, value):
    """A literal of `element` type for the numeric `value` (int or float)."""
    if element == "Integer":
        return str(int(value))
    text = f"{float(value)}"
    if element == "Float":
        return text
    return f"toFixed({text})"


# ---- helpers emitted once ---------------------------------------------------

HELPERS = r"""' Deterministic floor integer square root (Newton's method), n >= 0.
FUNC __vector_isqrtFloor(n AS Integer) AS Integer
  IF n <= 0 THEN
    RETURN 0
  END IF
  MUT x AS Integer = n
  MUT y AS Integer = (x + 1) / 2
  WHILE y < x
    x = y
    y = (x + n / x) / 2
  WEND
  RETURN x
END FUNC

' Integer square root rounded half away from zero (n >= 0). The exact half
' (f + 0.5)^2 = f^2 + f + 0.25 is never an integer, so there is no tie: round up
' exactly when the remainder exceeds the floor.
FUNC __vector_isqrtRound(n AS Integer) AS Integer
  LET f AS Integer = __vector_isqrtFloor(n)
  IF n - f * f > f THEN
    RETURN f + 1
  END IF
  RETURN f
END FUNC
"""


def fn(signature, body_lines):
    out = [f"FUNC {signature}"]
    out.extend("  " + line for line in body_lines)
    out.append("END FUNC")
    return "\n".join(out)


def sum_of_squares(var, dim):
    return " + ".join(f"{var}.{f} * {var}.{f}" for f in FIELDS[dim])


def gen_length(element, dim):
    t = tname(element, dim)
    sig = f"{impl('length', element, dim)}(v AS {t}) AS {element}"
    if element == "Integer":
        body = [
            f"LET s AS Integer = {sum_of_squares('v', dim)}",
            "RETURN __vector_isqrtRound(s)",
        ]
    else:
        body = [f"RETURN math::sqrt({sum_of_squares('v', dim)})"]
    return fn(sig, body)


def gen_dot(element, dim):
    t = tname(element, dim)
    sig = f"{impl('dot', element, dim)}(a AS {t}, b AS {t}) AS {element}"
    expr = " + ".join(f"a.{f} * b.{f}" for f in FIELDS[dim])
    return fn(sig, [f"RETURN {expr}"])


def gen_distance(element, dim):
    t = tname(element, dim)
    sig = f"{impl('distance', element, dim)}(a AS {t}, b AS {t}) AS {element}"
    body = [f"LET d{f} AS {element} = a.{f} - b.{f}" for f in FIELDS[dim]]
    ss = " + ".join(f"d{f} * d{f}" for f in FIELDS[dim])
    if element == "Integer":
        body.append(f"LET s AS Integer = {ss}")
        body.append("RETURN __vector_isqrtRound(s)")
    else:
        body.append(f"RETURN math::sqrt({ss})")
    return fn(sig, body)


def gen_normalize(element, dim):
    t = tname(element, dim)
    sig = f"{impl('normalize', element, dim)}(v AS {t}) AS {t}"
    msg = "vector::normalize of a zero-length vector"
    if element == "Integer":
        body = [
            f"LET s AS Integer = {sum_of_squares('v', dim)}",
            "IF s = 0 THEN",
            f'  FAIL error(77050002, "{msg}")',
            "END IF",
            "LET len AS Integer = __vector_isqrtRound(s)",
            "LET fl AS Float = toFloat(len)",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET r{f} AS Integer = math::round(toFloat(v.{f}) / fl)")
        body.append(f"RETURN {t}[{', '.join('r' + f for f in FIELDS[dim])}]")
    else:
        body = [
            f"LET len AS {element} = math::sqrt({sum_of_squares('v', dim)})",
            f"IF len = {zero(element)} THEN",
            f'  FAIL error(77050002, "{msg}")',
            "END IF",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS {element} = v.{f} / len")
        body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_cross(element, dim):
    t = tname(element, dim)
    if dim == 2:
        sig = f"{impl('cross', element, dim)}(v AS {t}) AS {t}"
        return fn(sig, [f"RETURN {t}[{zero(element)} - v.y, v.x]"])
    if dim == 3:
        sig = f"{impl('cross', element, dim)}(a AS {t}, b AS {t}) AS {t}"
        body = [
            f"LET cx AS {element} = a.y * b.z - a.z * b.y",
            f"LET cy AS {element} = a.z * b.x - a.x * b.z",
            f"LET cz AS {element} = a.x * b.y - a.y * b.x",
            f"RETURN {t}[cx, cy, cz]",
        ]
        return fn(sig, body)
    # dim == 4: ternary cofactor cross.
    sig = f"{impl('cross', element, dim)}(a AS {t}, b AS {t}, c AS {t}) AS {t}"
    body = [
        f"LET mZW AS {element} = b.z * c.w - b.w * c.z",
        f"LET mYW AS {element} = b.y * c.w - b.w * c.y",
        f"LET mYZ AS {element} = b.y * c.z - b.z * c.y",
        f"LET mXW AS {element} = b.x * c.w - b.w * c.x",
        f"LET mXZ AS {element} = b.x * c.z - b.z * c.x",
        f"LET mXY AS {element} = b.x * c.y - b.y * c.x",
        f"LET rx AS {element} = a.y * mZW - a.z * mYW + a.w * mYZ",
        f"LET ry AS {element} = a.z * mXW - a.x * mZW - a.w * mXZ",
        f"LET rz AS {element} = a.x * mYW - a.y * mXW + a.w * mXY",
        f"LET rw AS {element} = a.y * mXZ - a.x * mYZ - a.z * mXY",
        f"RETURN {t}[rx, ry, rz, rw]",
    ]
    return fn(sig, body)


def gen_reflect(element, dim):
    t = tname(element, dim)
    sig = f"{impl('reflect', element, dim)}(v AS {t}, n AS {t}) AS {t}"
    body = [
        f"LET d AS {element} = {impl('dot', element, dim)}(v, n)",
        f"LET k AS {element} = {lit(element, 2)} * d",
    ]
    for f in FIELDS[dim]:
        body.append(f"LET c{f} AS {element} = v.{f} - k * n.{f}")
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_project(element, dim):
    t = tname(element, dim)
    sig = f"{impl('project', element, dim)}(a AS {t}, b AS {t}) AS {t}"
    msg = "vector::project onto a zero-length vector"
    body = [
        f"LET db AS {element} = {impl('dot', element, dim)}(b, b)",
        f"IF db = {zero(element)} THEN",
        f'  FAIL error(77050002, "{msg}")',
        "END IF",
    ]
    if element == "Integer":
        body.append(
            f"LET ratio AS Float = toFloat({impl('dot', element, dim)}(a, b)) / toFloat(db)"
        )
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Integer = math::round(ratio * toFloat(b.{f}))")
    else:
        body.append(f"LET ratio AS {element} = {impl('dot', element, dim)}(a, b) / db")
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS {element} = ratio * b.{f}")
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_reject(element, dim):
    t = tname(element, dim)
    sig = f"{impl('reject', element, dim)}(a AS {t}, b AS {t}) AS {t}"
    body = [f"LET p AS {t} = {impl('project', element, dim)}(a, b)"]
    comps = ", ".join(f"a.{f} - p.{f}" for f in FIELDS[dim])
    body.append(f"RETURN {t}[{comps}]")
    return fn(sig, body)


def gen_angle_fixed_helper(dim):
    """Integer angle helper returning Fixed radians (no rounding); shared by the
    Integer angle and slerp overloads."""
    t = tname("Integer", dim)
    sig = f"__vector_angleFixed_integer{dim}(a AS {t}, b AS {t}) AS Fixed"
    msg = "vector::angle with a zero-length vector"
    body = [
        f"LET sa AS Integer = {impl('dot', 'Integer', dim)}(a, a)",
        f"LET sb AS Integer = {impl('dot', 'Integer', dim)}(b, b)",
        "IF sa = 0 OR sb = 0 THEN",
        f'  FAIL error(77050002, "{msg}")',
        "END IF",
        "LET la AS Fixed = math::sqrt(toFixed(sa))",
        "LET lb AS Fixed = math::sqrt(toFixed(sb))",
        f"LET cosv AS Fixed = toFixed({impl('dot', 'Integer', dim)}(a, b)) / (la * lb)",
        "LET clamped AS Fixed = math::clamp(cosv, toFixed(-1.0), toFixed(1.0))",
        "RETURN math::acos(clamped)",
    ]
    return fn(sig, body)


def gen_angle(element, dim):
    t = tname(element, dim)
    sig = f"{impl('angle', element, dim)}(a AS {t}, b AS {t}) AS {element}"
    if element == "Integer":
        return fn(sig, [f"RETURN math::round(__vector_angleFixed_integer{dim}(a, b))"])
    msg = "vector::angle with a zero-length vector"
    body = [
        f"LET la AS {element} = math::sqrt({impl('dot', element, dim)}(a, a))",
        f"LET lb AS {element} = math::sqrt({impl('dot', element, dim)}(b, b))",
        f"IF la = {zero(element)} OR lb = {zero(element)} THEN",
        f'  FAIL error(77050002, "{msg}")',
        "END IF",
        f"LET cosv AS {element} = {impl('dot', element, dim)}(a, b) / (la * lb)",
        f"LET clamped AS {element} = math::clamp(cosv, {lit(element, -1)}, {lit(element, 1)})",
        "RETURN math::acos(clamped)",
    ]
    return fn(sig, body)


def gen_lerp(element, dim, clamped):
    name = "lerp" if clamped else "lerp_unclamped"
    t = tname(element, dim)
    sig = f"{impl(name, element, dim)}(a AS {t}, b AS {t}, t AS Float) AS {t}"
    body = []
    tvar = "t"
    if clamped:
        body.append("LET tc AS Float = math::clamp(t, 0.0, 1.0)")
        tvar = "tc"
    if element == "Float":
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Float = a.{f} + (b.{f} - a.{f}) * {tvar}")
    elif element == "Fixed":
        body.append(f"LET tf AS Fixed = toFixed({tvar})")
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Fixed = a.{f} + (b.{f} - a.{f}) * tf")
    else:  # Integer
        for f in FIELDS[dim]:
            body.append(
                f"LET c{f} AS Integer = math::round(toFloat(a.{f}) + (toFloat(b.{f}) - toFloat(a.{f})) * {tvar})"
            )
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_slerp(element, dim):
    t = tname(element, dim)
    sig = f"{impl('slerp', element, dim)}(a AS {t}, b AS {t}, t AS Float) AS {t}"
    unclamped = impl("lerp_unclamped", element, dim)
    if element == "Float":
        body = [
            f"LET omega AS Float = {impl('angle', element, dim)}(a, b)",
            "LET s AS Float = math::sin(omega)",
            "IF math::abs(s) < 0.000001 THEN",
            f"  RETURN {unclamped}(a, b, t)",
            "END IF",
            "LET w0 AS Float = math::sin((1.0 - t) * omega) / s",
            "LET w1 AS Float = math::sin(t * omega) / s",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Float = w0 * a.{f} + w1 * b.{f}")
    elif element == "Fixed":
        body = [
            f"LET omega AS Fixed = {impl('angle', element, dim)}(a, b)",
            "LET s AS Fixed = math::sin(omega)",
            "IF math::abs(s) < toFixed(0.000001) THEN",
            f"  RETURN {unclamped}(a, b, t)",
            "END IF",
            "LET tf AS Fixed = toFixed(t)",
            "LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s",
            "LET w1 AS Fixed = math::sin(tf * omega) / s",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Fixed = w0 * a.{f} + w1 * b.{f}")
    else:  # Integer
        body = [
            f"LET omega AS Fixed = __vector_angleFixed_integer{dim}(a, b)",
            "LET s AS Fixed = math::sin(omega)",
            "IF math::abs(s) < toFixed(0.000001) THEN",
            f"  RETURN {unclamped}(a, b, t)",
            "END IF",
            "LET tf AS Fixed = toFixed(t)",
            "LET w0 AS Fixed = math::sin((toFixed(1.0) - tf) * omega) / s",
            "LET w1 AS Fixed = math::sin(tf * omega) / s",
        ]
        for f in FIELDS[dim]:
            body.append(
                f"LET c{f} AS Integer = math::round(w0 * toFixed(a.{f}) + w1 * toFixed(b.{f}))"
            )
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_clamp_length(element, dim):
    t = tname(element, dim)
    sig = f"{impl('clamp_length', element, dim)}(v AS {t}, maxLen AS {element}) AS {t}"
    msg_neg = "vector::clamp_length with negative max"
    body = [
        f"IF maxLen < {zero(element)} THEN",
        f'  FAIL error(77050002, "{msg_neg}")',
        "END IF",
    ]
    if element == "Integer":
        body += [
            f"LET s AS Integer = {sum_of_squares('v', dim)}",
            "LET len AS Integer = __vector_isqrtRound(s)",
            "IF len <= maxLen OR len = 0 THEN",
            "  RETURN v",
            "END IF",
            "LET ratio AS Float = toFloat(maxLen) / toFloat(len)",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS Integer = math::round(toFloat(v.{f}) * ratio)")
    else:
        body += [
            f"LET len AS {element} = math::sqrt({sum_of_squares('v', dim)})",
            f"IF len <= maxLen OR len = {zero(element)} THEN",
            "  RETURN v",
            "END IF",
            f"LET ratio AS {element} = maxLen / len",
        ]
        for f in FIELDS[dim]:
            body.append(f"LET c{f} AS {element} = v.{f} * ratio")
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_scale(element, dim):
    t = tname(element, dim)
    sig = f"{impl('scale', element, dim)}(a AS {t}, b AS {t}) AS {t}"
    body = [f"LET c{f} AS {element} = a.{f} * b.{f}" for f in FIELDS[dim]]
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_minmax(name, element, dim):
    t = tname(element, dim)
    sig = f"{impl(name, element, dim)}(a AS {t}, b AS {t}) AS {t}"
    body = [f"LET c{f} AS {element} = math::{name}(a.{f}, b.{f})" for f in FIELDS[dim]]
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_abs(element, dim):
    t = tname(element, dim)
    sig = f"{impl('abs', element, dim)}(v AS {t}) AS {t}"
    body = [f"LET c{f} AS {element} = math::abs(v.{f})" for f in FIELDS[dim]]
    body.append(f"RETURN {t}[{', '.join('c' + f for f in FIELDS[dim])}]")
    return fn(sig, body)


def gen_perpendicular(element):
    t = tname(element, 2)
    sig = f"{impl('perpendicular', element, 2)}(v AS {t}) AS {t}"
    return fn(sig, [f"RETURN {t}[{zero(element)} - v.y, v.x]"])


def gen_rotate_2d(element):
    t = tname(element, 2)
    sig = f"{impl('rotate_2d', element, 2)}(v AS {t}, ang AS Float) AS {t}"
    if element == "Float":
        body = [
            "LET c AS Float = math::cos(ang)",
            "LET s AS Float = math::sin(ang)",
            "LET rx AS Float = v.x * c - v.y * s",
            "LET ry AS Float = v.x * s + v.y * c",
            f"RETURN {t}[rx, ry]",
        ]
    elif element == "Fixed":
        body = [
            "LET af AS Fixed = toFixed(ang)",
            "LET c AS Fixed = math::cos(af)",
            "LET s AS Fixed = math::sin(af)",
            "LET rx AS Fixed = v.x * c - v.y * s",
            "LET ry AS Fixed = v.x * s + v.y * c",
            f"RETURN {t}[rx, ry]",
        ]
    else:
        body = [
            "LET af AS Fixed = toFixed(ang)",
            "LET c AS Fixed = math::cos(af)",
            "LET s AS Fixed = math::sin(af)",
            "LET xf AS Fixed = toFixed(v.x)",
            "LET yf AS Fixed = toFixed(v.y)",
            "LET rx AS Integer = math::round(xf * c - yf * s)",
            "LET ry AS Integer = math::round(xf * s + yf * c)",
            f"RETURN {t}[rx, ry]",
        ]
    return fn(sig, body)


def gen_tostring(element, dim):
    t = tname(element, dim)
    sig = f"__vector_toString_{element.lower()}{dim}(v AS {t}) AS String"
    parts = ['"("']
    for i, f in enumerate(FIELDS[dim]):
        if i > 0:
            parts.append('", "')
        parts.append(f"toString(v.{f})")
    parts.append('")"')
    return fn(sig, [f"RETURN {' & '.join(parts)}"])


def gen_type(element, dim):
    out = [f"EXPORT TYPE {tname(element, dim)}"]
    for f in FIELDS[dim]:
        out.append(f"  {f} AS {element}")
    out.append("END TYPE")
    return "\n".join(out)


def main():
    blocks = []
    header = """' Source companion for the built-in `vector` package (plan-06-vector.md).
'
' GENERATED by scripts/gen_vector_package.py — do not edit by hand; edit the
' generator and re-run it. Nine fixed-width math-vector value records and the
' overloaded geometry/utility/2D functions over them. All evaluation is in the
' canonical left-to-right order of plan-06 §4; algebraic ops are
' correctly-rounded (FSQRT/IEEE), Fixed is Q32.32, Integer uses a rounding
' integer square root and rounds half away from zero (math::round) for every
' result derived from a real-valued computation. The three trig members route
' to math::'s deterministic Fixed/Float kernels.
IMPORT math"""
    blocks.append(header)

    # Types.
    for element in ELEMENTS:
        for dim in DIMS:
            blocks.append(gen_type(element, dim))

    blocks.append("' ---- shared integer helpers ----")
    blocks.append(HELPERS.strip())

    # Integer angle-as-Fixed helpers (shared by angle + slerp).
    blocks.append("' ---- integer angle (Fixed radians) helpers ----")
    for dim in DIMS:
        blocks.append(gen_angle_fixed_helper(dim))

    # Core + derived + interpolation + utility functions.
    section_gens = [
        ("core geometry", lambda e, d: [gen_length(e, d), gen_dot(e, d), gen_distance(e, d)]),
        ("normalize + cross", lambda e, d: [gen_normalize(e, d), gen_cross(e, d)]),
        (
            "derived geometry",
            lambda e, d: [gen_reflect(e, d), gen_project(e, d), gen_reject(e, d), gen_angle(e, d)],
        ),
        (
            "interpolation + magnitude",
            lambda e, d: [
                gen_lerp(e, d, True),
                gen_lerp(e, d, False),
                gen_slerp(e, d),
                gen_clamp_length(e, d),
            ],
        ),
        (
            "component-wise utilities",
            lambda e, d: [
                gen_scale(e, d),
                gen_minmax("min", e, d),
                gen_minmax("max", e, d),
                gen_abs(e, d),
            ],
        ),
        ("presentation", lambda e, d: [gen_tostring(e, d)]),
    ]
    for title, gen in section_gens:
        blocks.append(f"' ---- {title} ----")
        for element in ELEMENTS:
            for dim in DIMS:
                for piece in gen(element, dim):
                    blocks.append(piece)

    # 2D-only members.
    blocks.append("' ---- 2D-only members ----")
    for element in ELEMENTS:
        blocks.append(gen_perpendicular(element))
        blocks.append(gen_rotate_2d(element))

    text = "\n\n".join(blocks) + "\n"
    sys.stdout.write(text)


if __name__ == "__main__":
    main()
