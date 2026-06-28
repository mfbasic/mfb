#!/usr/bin/env python3
"""Generate the tests/func_vector_* acceptance fixtures for plan-06-vector.

For every function member: one `func_vector_<fn>_valid` exercising all of its
type/arity overloads with deterministic inputs, and one `func_vector_<fn>_invalid`
exercising wrong arity / mismatched-or-non-vector argument types. Plus the
runtime-trap `_rt` fixtures and the Phase-1 type/constant fixtures.

Golden files (.ast/.ir/.run/build.log) are created empty here and then filled by
scripts/sync-goldens.sh, so this script only writes project.json + src + empty
golden placeholders.
"""

import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
TESTS = ROOT / "tests"

ELEMENTS = ["Float", "Fixed", "Integer"]
DIMS = [2, 3, 4]
FIELDS = {2: ["x", "y"], 3: ["x", "y", "z"], 4: ["x", "y", "z", "w"]}

# Deterministic, non-degenerate sample components per dimension.
A_VALS = {2: [3, 4], 3: [1, 2, 2], 4: [1, 2, 2, 4]}
B_VALS = {2: [1, 2], 3: [2, 1, 2], 4: [2, 1, 0, 1]}
C_VALS = {4: [0, 1, 1, 0]}


def tname(element, dim):
    return f"vector::{element}{dim}"


def scalar_lit(element, value):
    if element == "Integer":
        return str(int(value))
    if element == "Float":
        return f"{float(value)}"
    return f"toFixed({float(value)})"


def vec_lit(element, dim, vals):
    comps = ", ".join(scalar_lit(element, v) for v in vals[:dim])
    return f"{tname(element, dim)}[{comps}]"


PROJECT_JSON = """{{
  "name": "{name}",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{{ "root": "src", "role": "main", "include": ["**/*.mfb"] }}],
  "entry": "main",
  "targets": ["native"]
}}
"""


def write_test(name, source, golden_files):
    d = TESTS / name
    (d / "src").mkdir(parents=True, exist_ok=True)
    (d / "golden").mkdir(parents=True, exist_ok=True)
    (d / "project.json").write_text(PROJECT_JSON.format(name=name))
    (d / "src" / "main.mfb").write_text(source)
    for gf in golden_files:
        gp = d / "golden" / gf
        if not gp.exists():
            gp.write_text("")


def valid_golden(name):
    return ["build.log", f"{name}.ast", f"{name}.ir", f"{name}.run"]


def header(extra_imports=()):
    lines = ["IMPORT vector", "IMPORT io"]
    for imp in extra_imports:
        lines.append(f"IMPORT {imp}")
    return "\n".join(lines) + "\n\n"


def label(fn, element, dim):
    return f"{fn} {element}{dim}"


# Per-function call-expression builders. Each returns the call source given the
# overload's (element, dim). Functions list which (element, dim) overloads exist.
def overloads_all():
    return [(e, d) for e in ELEMENTS for d in DIMS]


def overloads_2d():
    return [(e, 2) for e in ELEMENTS]


def call_unary(fn, element, dim):
    return f"vector::{fn}({vec_lit(element, dim, A_VALS[dim])})"


def call_binary(fn, element, dim):
    return f"vector::{fn}({vec_lit(element, dim, A_VALS[dim])}, {vec_lit(element, dim, B_VALS[dim])})"


def call_cross(element, dim):
    a = vec_lit(element, dim, A_VALS[dim])
    if dim == 2:
        return f"vector::cross({a})"
    if dim == 3:
        return f"vector::cross({a}, {vec_lit(element, dim, B_VALS[dim])})"
    return (
        f"vector::cross({a}, {vec_lit(element, dim, B_VALS[dim])}, "
        f"{vec_lit(element, dim, C_VALS[dim])})"
    )


def call_lerp(fn, element, dim, t):
    a = vec_lit(element, dim, A_VALS[dim])
    b = vec_lit(element, dim, B_VALS[dim])
    return f"vector::{fn}({a}, {b}, {t})"


def call_clamp_length(element, dim, m):
    return f"vector::clamp_length({vec_lit(element, dim, A_VALS[dim])}, {scalar_lit(element, m)})"


def call_rotate(element, t):
    return f"vector::rotate_2d({vec_lit(element, 2, A_VALS[2])}, {t})"


def emit_valid(fn, overloads, call_for, imports=()):
    name = f"func_vector_{fn}_valid"
    body = [header(imports), "FUNC main AS Integer"]
    for (element, dim) in overloads:
        call = call_for(element, dim)
        body.append(f'  io::print("{label(fn, element, dim)} = " & toString({call}))')
    body.append("  RETURN 0")
    body.append("END FUNC")
    write_test(name, "\n".join(body) + "\n", valid_golden(name))


def emit_invalid(fn, lines, imports=()):
    name = f"func_vector_{fn}_invalid"
    body = [header(imports), "FUNC main AS Integer"]
    for i, line in enumerate(lines):
        body.append(f"  {line}")
    body.append("  RETURN 0")
    body.append("END FUNC")
    write_test(name, "\n".join(body) + "\n", ["build.log"])


def main():
    # ---- valid fixtures, one per function -----------------------------------
    emit_valid("length", overloads_all(), lambda e, d: call_unary("length", e, d))
    emit_valid("normalize", overloads_all(), lambda e, d: call_unary("normalize", e, d))
    emit_valid("abs", overloads_all(),
               lambda e, d: f"vector::abs({vec_lit(e, d, [-3, 4, -5, 6])})")
    emit_valid("distance", overloads_all(), lambda e, d: call_binary("distance", e, d))
    emit_valid("dot", overloads_all(), lambda e, d: call_binary("dot", e, d))
    emit_valid("cross", overloads_all(), call_cross)
    emit_valid("reflect", overloads_all(), lambda e, d: call_binary("reflect", e, d))
    emit_valid("project", overloads_all(), lambda e, d: call_binary("project", e, d))
    emit_valid("reject", overloads_all(), lambda e, d: call_binary("reject", e, d))
    emit_valid("angle", overloads_all(), lambda e, d: call_binary("angle", e, d))
    emit_valid("scale", overloads_all(), lambda e, d: call_binary("scale", e, d))
    emit_valid("min", overloads_all(), lambda e, d: call_binary("min", e, d))
    emit_valid("max", overloads_all(), lambda e, d: call_binary("max", e, d))

    # lerp / lerp_unclamped show the clamp split (t = 2.0 extrapolates).
    def lerp_valid(fn):
        name = f"func_vector_{fn}_valid"
        body = [header(), "FUNC main AS Integer"]
        for (element, dim) in overloads_all():
            for t in ["0.5", "2.0"]:
                call = call_lerp(fn, element, dim, t)
                body.append(
                    f'  io::print("{fn} {element}{dim} t={t} = " & toString({call}))'
                )
        body.append("  RETURN 0")
        body.append("END FUNC")
        write_test(name, "\n".join(body) + "\n", valid_golden(name))

    lerp_valid("lerp")
    lerp_valid("lerp_unclamped")

    # slerp over unit-ish vectors.
    emit_valid("slerp", overloads_all(),
               lambda e, d: call_lerp("slerp", e, d, "0.5"))

    emit_valid("clamp_length", overloads_all(),
               lambda e, d: call_clamp_length(e, d, 2))
    emit_valid("perpendicular", overloads_2d(),
               lambda e, d: f"vector::perpendicular({vec_lit(e, 2, A_VALS[2])})")
    emit_valid("rotate_2d", overloads_2d(),
               lambda e, d: call_rotate(e, "math::pi2"), imports=("math",))

    # toString override over all nine types.
    name = "func_vector_toString_valid"
    body = [header(), "FUNC main AS Integer"]
    for (element, dim) in overloads_all():
        body.append(
            f'  io::print(toString({vec_lit(element, dim, A_VALS[dim])}))'
        )
    body.append("  RETURN 0")
    body.append("END FUNC")
    write_test(name, "\n".join(body) + "\n", valid_golden(name))

    # ---- Phase-1 fixtures: construction/copy/field + constants --------------
    name = "func_vector_types_valid"
    body = [header(), "FUNC main AS Integer"]
    for (element, dim) in overloads_all():
        v = vec_lit(element, dim, A_VALS[dim])
        body.append(f"  LET v{element}{dim} AS {tname(element, dim)} = {v}")
        accesses = " & \",\" & ".join(f"toString(v{element}{dim}.{f})" for f in FIELDS[dim])
        body.append(f'  io::print("{element}{dim}: " & {accesses})')
    body.append("  RETURN 0")
    body.append("END FUNC")
    write_test(name, "\n".join(body) + "\n", valid_golden(name))

    name = "func_vector_constants_valid"
    body = [header(), "FUNC main AS Integer"]
    bases = ["zero", "one", "up", "right", "forward"]
    for base in bases:
        for element in ELEMENTS:
            for dim in DIMS:
                if base == "forward" and dim < 3:
                    continue
                const = f"vector::{base}{element}{dim}"
                body.append(f'  io::print("{base}{element}{dim} = " & toString({const}))')
    body.append("  RETURN 0")
    body.append("END FUNC")
    write_test(name, "\n".join(body) + "\n", valid_golden(name))

    # ---- invalid fixtures ---------------------------------------------------
    emit_invalid("length", [
        "LET a AS Float = vector::length(vector::Float3[1.0,2.0,3.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS Float = vector::length(5.0)",
        "LET c AS Float = vector::length(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("dot", [
        "LET a AS Float = vector::dot(vector::Float2[1.0,2.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS Float = vector::dot(vector::Float2[1.0,2.0])",
        "LET c AS Float = vector::dot(1.0, 2.0)",
    ])
    emit_invalid("distance", [
        "LET a AS Float = vector::distance(vector::Float2[1.0,2.0], vector::Integer2[1,2])",
        "LET b AS Float = vector::distance(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("normalize", [
        "LET a AS vector::Float3 = vector::normalize(5.0)",
        "LET b AS vector::Float3 = vector::normalize(vector::Float3[1.0,2.0,3.0], vector::Float3[1.0,2.0,3.0])",
    ])
    emit_invalid("cross", [
        "LET a AS vector::Float2 = vector::cross(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
        "LET b AS vector::Float3 = vector::cross(vector::Float3[1.0,2.0,3.0])",
        "LET c AS vector::Float4 = vector::cross(vector::Float4[1.0,2.0,3.0,4.0], vector::Float4[1.0,2.0,3.0,4.0])",
    ])
    emit_invalid("reflect", [
        "LET a AS vector::Float2 = vector::reflect(vector::Float2[1.0,2.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS vector::Float2 = vector::reflect(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("project", [
        "LET a AS vector::Float2 = vector::project(vector::Float2[1.0,2.0], vector::Integer2[1,2])",
        "LET b AS vector::Float2 = vector::project(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("reject", [
        "LET a AS vector::Float2 = vector::reject(vector::Float2[1.0,2.0], vector::Integer2[1,2])",
        "LET b AS vector::Float2 = vector::reject(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("angle", [
        "LET a AS Float = vector::angle(vector::Float2[1.0,2.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS Float = vector::angle(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("lerp", [
        "LET a AS vector::Float2 = vector::lerp(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
        "LET b AS vector::Float2 = vector::lerp(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0], 1)",
        "LET c AS vector::Float2 = vector::lerp(vector::Float2[1.0,2.0], vector::Integer2[3,4], 0.5)",
    ])
    emit_invalid("lerp_unclamped", [
        "LET a AS vector::Float2 = vector::lerp_unclamped(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
        "LET b AS vector::Float2 = vector::lerp_unclamped(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0], 1)",
    ])
    emit_invalid("slerp", [
        "LET a AS vector::Float2 = vector::slerp(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
        "LET b AS vector::Float2 = vector::slerp(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0], 1)",
    ])
    emit_invalid("clamp_length", [
        "LET a AS vector::Fixed2 = vector::clamp_length(vector::Fixed2[toFixed(1.0),toFixed(2.0)], 2.0)",
        "LET b AS vector::Float2 = vector::clamp_length(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("scale", [
        "LET a AS vector::Float2 = vector::scale(vector::Float2[1.0,2.0], vector::Integer2[1,2])",
        "LET b AS vector::Float2 = vector::scale(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("min", [
        "LET a AS vector::Float2 = vector::min(vector::Float2[1.0,2.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS vector::Float2 = vector::min(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("max", [
        "LET a AS vector::Float2 = vector::max(vector::Float2[1.0,2.0], vector::Float3[1.0,2.0,3.0])",
        "LET b AS vector::Float2 = vector::max(vector::Float2[1.0,2.0])",
    ])
    emit_invalid("abs", [
        "LET a AS vector::Float2 = vector::abs(5.0)",
        "LET b AS vector::Float2 = vector::abs(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
    ])
    emit_invalid("perpendicular", [
        "LET a AS vector::Float3 = vector::perpendicular(vector::Float3[1.0,2.0,3.0])",
        "LET b AS vector::Float2 = vector::perpendicular(vector::Float2[1.0,2.0], vector::Float2[3.0,4.0])",
    ])
    emit_invalid("rotate_2d", [
        "LET a AS vector::Float3 = vector::rotate_2d(vector::Float3[1.0,2.0,3.0], 1.0)",
        "LET b AS vector::Float2 = vector::rotate_2d(vector::Float2[1.0,2.0])",
        "LET c AS vector::Float2 = vector::rotate_2d(vector::Float2[1.0,2.0], 1)",
    ])
    # forward undefined in 2D; mismatched constant type.
    emit_invalid("constants", [
        "LET a AS vector::Float2 = vector::forwardFloat2",
        "LET b AS vector::Integer2 = vector::zeroFloat2",
    ])

    # ---- runtime-trap fixtures (_rt) ----------------------------------------
    rt_cases = {
        "normalize_zero_rt": "toString(vector::normalize(vector::Float3[0.0, 0.0, 0.0]))",
        "project_zero_rt": "toString(vector::project(vector::Float2[1.0, 1.0], vector::Float2[0.0, 0.0]))",
        "reject_zero_rt": "toString(vector::reject(vector::Float2[1.0, 1.0], vector::Float2[0.0, 0.0]))",
        "angle_zero_rt": "toString(vector::angle(vector::Float2[0.0, 0.0], vector::Float2[1.0, 0.0]))",
        "slerp_zero_rt": "toString(vector::slerp(vector::Float2[0.0, 0.0], vector::Float2[1.0, 0.0], 0.5))",
        "clamp_length_negative_rt": "toString(vector::clamp_length(vector::Float2[1.0, 1.0], 0.0 - 1.0))",
        "normalize_zero_integer_rt": "toString(vector::normalize(vector::Integer2[0, 0]))",
    }
    for suffix, expr in rt_cases.items():
        name = f"func_vector_{suffix}"
        src = header() + "FUNC main AS Integer\n  io::print(" + expr + ")\n  RETURN 0\nEND FUNC\n"
        write_test(name, src, valid_golden(name))

    print("vector test fixtures written")


if __name__ == "__main__":
    main()
