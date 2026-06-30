#!/usr/bin/env python3
"""Generate the per-function man pages under src/docs/man/builtins/vector/.

package.txt and types.txt are hand-written; this emits the 19 function pages.
These pages use the legacy plain-text man layout (NAME / SYNOPSIS / PACKAGE /
IMPORTS / DESCRIPTION / PARAMETERS / RETURN VALUE / ERRORS / EXAMPLES / SEE
ALSO), which the renderer still prints verbatim. New/updated pages should follow
the Markdown template in .ai/man_template.md instead; this generator has not yet
been ported to it.
"""

import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "src/docs/man/builtins/vector"

VEC = "a vector (Float2/3/4, Fixed2/3/4, or Integer2/3/4)"
ERR_INVALID = "  77050002 (ErrInvalidArgument)\n"
ERR_OVERFLOW = (
    "  77050010 (ErrOverflow)\n"
    "  Raised when an Integer or Fixed result exceeds the representable range,\n"
    "  as in the scalar math:: functions.\n"
)

# Each entry: signatures (list), one-liner, description, params (list of
# (name, type, meaning)), ret (type, meaning), errors (text or None),
# example caption, example code, see-also.
FUNCS = {
    "length": {
        "sig": ["vector::length(v AS T_N) AS T"],
        "one": "Euclidean length (magnitude) of a vector",
        "desc": (
            "Returns the Euclidean length sqrt(x*x + y*y + ...) of v, summed in\n"
            "  declared component order. For Float the square root is the hardware\n"
            "  FSQRT; for Fixed it is the deterministic Q32.32 square root; for\n"
            "  Integer it is a deterministic integer square root of the squared sum,\n"
            "  rounded half away from zero (so it is exact for large sums with no\n"
            "  floating-point round-trip). The return type is the element type T."
        ),
        "params": [("v", "T_N", "the vector to measure")],
        "ret": ("T", "the magnitude, in the element type"),
        "errors": None,
        "ex_cap": "Length of a 3-4-5 vector",
        "ex": 'io::print(toString(vector::length(vector::Float3[3.0, 0.0, 4.0])))',
        "see": "distance, normalize, dot",
    },
    "normalize": {
        "sig": ["vector::normalize(v AS T_N) AS T_N"],
        "one": "unit vector in the same direction",
        "desc": (
            "Returns a vector of length 1 pointing the same way as v: each\n"
            "  component divided by length(v). Float and Fixed use correctly-rounded\n"
            "  division; Integer rounds each quotient half away from zero, which is\n"
            "  intentionally lossy (most Integer unit vectors land in -1, 0, 1). A\n"
            "  zero-length v has no direction and raises ErrInvalidArgument."
        ),
        "params": [("v", "T_N", "the vector to normalize; must be non-zero")],
        "ret": ("T_N", "the unit vector"),
        "errors": ERR_INVALID + "  Raised when v has zero length.\n",
        "ex_cap": "Normalize to a unit vector",
        "ex": 'io::print(toString(vector::normalize(vector::Float3[3.0, 0.0, 4.0])))',
        "see": "length, clamp_length",
    },
    "distance": {
        "sig": ["vector::distance(a AS T_N, b AS T_N) AS T"],
        "one": "distance between two points",
        "desc": (
            "Returns length(a - b): the Euclidean distance between the two points,\n"
            "  with the same per-element-type rules and Integer rounding as length."
        ),
        "params": [
            ("a", "T_N", "the first point"),
            ("b", "T_N", "the second point, same type as a"),
        ],
        "ret": ("T", "the distance, in the element type"),
        "errors": None,
        "ex_cap": "Distance between two 2D points",
        "ex": 'io::print(toString(vector::distance(vector::Float2[0.0, 0.0], vector::Float2[3.0, 4.0])))',
        "see": "length",
    },
    "dot": {
        "sig": ["vector::dot(a AS T_N, b AS T_N) AS T"],
        "one": "dot (inner) product",
        "desc": (
            "Returns the dot product a.x*b.x + a.y*b.y + ..., summed in declared\n"
            "  order. Exact integer arithmetic for Integer (no rounding); Float and\n"
            "  Fixed use their arithmetic. The sign tells you whether the vectors\n"
            "  point roughly the same way (positive) or opposite (negative)."
        ),
        "params": [
            ("a", "T_N", "the first vector"),
            ("b", "T_N", "the second vector, same type as a"),
        ],
        "ret": ("T", "the dot product, in the element type"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Dot product of two vectors",
        "ex": 'io::print(toString(vector::dot(vector::Float3[1.0, 2.0, 3.0], vector::Float3[4.0, 5.0, 6.0])))',
        "see": "cross, angle, project",
    },
    "cross": {
        "sig": [
            "vector::cross(v AS T2) AS T2",
            "vector::cross(a AS T3, b AS T3) AS T3",
            "vector::cross(a AS T4, b AS T4, c AS T4) AS T4",
        ],
        "one": "generalized (n-1)-ary cross product",
        "desc": (
            "Returns the cross product: the vector perpendicular to its operands.\n"
            "  This is the generalized (n-1)-ary form, so its arity is\n"
            "  dimension-specific. In 2D it is unary and returns the left\n"
            "  perpendicular (-v.y, v.x) (a 90-degree counterclockwise rotation). In\n"
            "  3D it is the binary a x b. In 4D it is ternary: the vector\n"
            "  perpendicular to all three operands, via the cofactor determinant.\n"
            "  Exact for every element type (no rounding)."
        ),
        "params": [
            ("v / a", "T_N", "the operand(s); one in 2D, two in 3D, three in 4D"),
        ],
        "ret": ("T_N", "the perpendicular vector"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "3D cross product of two basis vectors",
        "ex": 'io::print(toString(vector::cross(vector::Float3[1.0,0.0,0.0], vector::Float3[0.0,1.0,0.0])))',
        "see": "perpendicular, dot",
    },
    "reflect": {
        "sig": ["vector::reflect(v AS T_N, n AS T_N) AS T_N"],
        "one": "reflect a vector about a normal",
        "desc": (
            "Returns v - 2*dot(v, n)*n: the reflection of v across the hyperplane\n"
            "  with normal n. n is taken as given and is NOT normalized -- pass a\n"
            "  unit normal for a length-preserving reflection. Exact for every\n"
            "  element type (multiply and subtract only)."
        ),
        "params": [
            ("v", "T_N", "the incoming vector"),
            ("n", "T_N", "the surface normal (caller supplies a unit normal)"),
        ],
        "ret": ("T_N", "the reflected vector"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Reflect about the +y axis",
        "ex": 'io::print(toString(vector::reflect(vector::Float2[1.0,-1.0], vector::Float2[0.0,1.0])))',
        "see": "project, reject",
    },
    "project": {
        "sig": ["vector::project(a AS T_N, b AS T_N) AS T_N"],
        "one": "vector projection of a onto b",
        "desc": (
            "Returns (dot(a, b) / dot(b, b)) * b: the component of a that lies along\n"
            "  b. b must be non-zero. Float and Fixed use correctly-rounded division;\n"
            "  Integer rounds each component half away from zero (intentionally\n"
            "  lossy)."
        ),
        "params": [
            ("a", "T_N", "the vector to project"),
            ("b", "T_N", "the direction to project onto; must be non-zero"),
        ],
        "ret": ("T_N", "the projection of a onto b"),
        "errors": ERR_INVALID + "  Raised when b has zero length.\n",
        "ex_cap": "Project onto the +x axis",
        "ex": 'io::print(toString(vector::project(vector::Float2[2.0,2.0], vector::Float2[1.0,0.0])))',
        "see": "reject, reflect, dot",
    },
    "reject": {
        "sig": ["vector::reject(a AS T_N, b AS T_N) AS T_N"],
        "one": "component of a orthogonal to b",
        "desc": (
            "Returns a - project(a, b): the part of a perpendicular to b. Same\n"
            "  zero-b error and per-type rounding as project; for Integer, project +\n"
            "  reject round-trips as closely as integers allow."
        ),
        "params": [
            ("a", "T_N", "the vector to decompose"),
            ("b", "T_N", "the direction to remove; must be non-zero"),
        ],
        "ret": ("T_N", "the orthogonal component"),
        "errors": ERR_INVALID + "  Raised when b has zero length.\n",
        "ex_cap": "Orthogonal component relative to the +x axis",
        "ex": 'io::print(toString(vector::reject(vector::Float2[2.0,2.0], vector::Float2[1.0,0.0])))',
        "see": "project, reflect",
    },
    "angle": {
        "sig": ["vector::angle(a AS T_N, b AS T_N) AS T"],
        "one": "unsigned angle between two vectors in radians",
        "desc": (
            "Returns the unsigned angle in radians between a and b:\n"
            "  acos(clamp(dot(a, b) / (length(a)*length(b)), -1, 1)). The cosine is\n"
            "  clamped to [-1, 1] before acos so rounding can never push it out of\n"
            "  domain -- the function is total for any two non-zero vectors. Float\n"
            "  uses the in-tree Float acos; Fixed and Integer use the deterministic\n"
            "  Q32.32 acos, and Integer rounds the radian result to an Integer\n"
            "  (degenerate, 0..3). Either input zero-length raises ErrInvalidArgument."
        ),
        "params": [
            ("a", "T_N", "the first vector; must be non-zero"),
            ("b", "T_N", "the second vector, same type; must be non-zero"),
        ],
        "ret": ("T", "the angle in radians, in the element type"),
        "errors": ERR_INVALID + "  Raised when either input has zero length.\n",
        "ex_cap": "Right angle between the axes",
        "ex": 'io::print(toString(vector::angle(vector::Float2[1.0,0.0], vector::Float2[0.0,1.0])))',
        "see": "dot, slerp",
    },
    "lerp": {
        "sig": ["vector::lerp(a AS T_N, b AS T_N, t AS Float) AS T_N"],
        "one": "clamped linear interpolation",
        "desc": (
            "Returns a + (b - a)*t component-wise, with t first clamped to [0, 1] so\n"
            "  the result always lies on the segment from a to b. t is a Float for\n"
            "  every element type; for Fixed it is converted with toFixed after the\n"
            "  clamp, and for Integer the interpolation is computed in Float and each\n"
            "  component is rounded half away from zero. Use lerp_unclamped to\n"
            "  extrapolate beyond the endpoints."
        ),
        "params": [
            ("a", "T_N", "the start vector (t = 0)"),
            ("b", "T_N", "the end vector (t = 1), same type as a"),
            ("t", "Float", "interpolation parameter, clamped to [0, 1]"),
        ],
        "ret": ("T_N", "the interpolated vector"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Midpoint, and a clamped overshoot",
        "ex": (
            'io::print(toString(vector::lerp(vector::Float2[0.0,0.0], vector::Float2[10.0,0.0], 0.5)))\n'
            '  io::print(toString(vector::lerp(vector::Float2[0.0,0.0], vector::Float2[10.0,0.0], 2.0)))'
        ),
        "see": "lerp_unclamped, slerp",
    },
    "lerp_unclamped": {
        "sig": ["vector::lerp_unclamped(a AS T_N, b AS T_N, t AS Float) AS T_N"],
        "one": "linear interpolation, extrapolating outside [0, 1]",
        "desc": (
            "Like lerp but uses t verbatim with no clamp, so t outside [0, 1]\n"
            "  extrapolates past the endpoints. Same per-element-type rules and\n"
            "  Integer rounding as lerp."
        ),
        "params": [
            ("a", "T_N", "the start vector (t = 0)"),
            ("b", "T_N", "the end vector (t = 1), same type as a"),
            ("t", "Float", "interpolation parameter, not clamped"),
        ],
        "ret": ("T_N", "the interpolated or extrapolated vector"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Extrapolate beyond the endpoint",
        "ex": 'io::print(toString(vector::lerp_unclamped(vector::Float2[0.0,0.0], vector::Float2[10.0,0.0], 2.0)))',
        "see": "lerp, slerp",
    },
    "slerp": {
        "sig": ["vector::slerp(a AS T_N, b AS T_N, t AS Float) AS T_N"],
        "one": "spherical linear interpolation",
        "desc": (
            "Interpolates along the great-circle arc from a to b: with omega =\n"
            "  angle(a, b) and s = sin(omega), the result is\n"
            "  (sin((1-t)*omega)/s)*a + (sin(t*omega)/s)*b, with t unclamped. Near the\n"
            "  degenerate parallel or antiparallel poles (sin(omega) ~ 0) it falls\n"
            "  back to lerp_unclamped to stay numerically stable. slerp interpolates\n"
            "  direction; it preserves magnitude only when length(a) == length(b).\n"
            "  The trig is deterministic (Fixed Q32.32 or in-tree Float). Either\n"
            "  input zero-length raises ErrInvalidArgument."
        ),
        "params": [
            ("a", "T_N", "the start vector; must be non-zero"),
            ("b", "T_N", "the end vector, same type; must be non-zero"),
            ("t", "Float", "interpolation parameter, not clamped"),
        ],
        "ret": ("T_N", "the spherically interpolated vector"),
        "errors": ERR_INVALID + "  Raised when either input has zero length.\n",
        "ex_cap": "Halfway along the arc between the axes",
        "ex": 'io::print(toString(vector::slerp(vector::Float2[1.0,0.0], vector::Float2[0.0,1.0], 0.5)))',
        "see": "lerp, lerp_unclamped, angle",
    },
    "clamp_length": {
        "sig": ["vector::clamp_length(v AS T_N, max AS T) AS T_N"],
        "one": "cap a vector's magnitude, preserving direction",
        "desc": (
            "Caps the magnitude of v at max, leaving its direction unchanged: if\n"
            "  length(v) <= max (or v is zero) v is returned unchanged, otherwise\n"
            "  each component is scaled by max/length(v). max is a scalar of the\n"
            "  element type and must be non-negative. Integer scaling rounds each\n"
            "  component half away from zero."
        ),
        "params": [
            ("v", "T_N", "the vector to cap"),
            ("max", "T", "the maximum magnitude; must be >= 0"),
        ],
        "ret": ("T_N", "v unchanged if within max, else v scaled to length max"),
        "errors": ERR_INVALID + "  Raised when max is negative.\n",
        "ex_cap": "Cap a length-5 vector at 2.5",
        "ex": 'io::print(toString(vector::clamp_length(vector::Float2[3.0,4.0], 2.5)))',
        "see": "length, normalize",
    },
    "scale": {
        "sig": ["vector::scale(a AS T_N, b AS T_N) AS T_N"],
        "one": "component-wise (Hadamard) product",
        "desc": (
            "Returns the component-wise product (a.x*b.x, a.y*b.y, ...). Exact for\n"
            "  every element type."
        ),
        "params": [
            ("a", "T_N", "the first vector"),
            ("b", "T_N", "the second vector, same type as a"),
        ],
        "ret": ("T_N", "the component-wise product"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Component-wise scaling",
        "ex": 'io::print(toString(vector::scale(vector::Float2[2.0,3.0], vector::Float2[4.0,5.0])))',
        "see": "min, max, abs",
    },
    "min": {
        "sig": ["vector::min(a AS T_N, b AS T_N) AS T_N"],
        "one": "component-wise minimum",
        "desc": "Returns the component-wise minimum, each component math::min(a.i, b.i).",
        "params": [
            ("a", "T_N", "the first vector"),
            ("b", "T_N", "the second vector, same type as a"),
        ],
        "ret": ("T_N", "the component-wise minimum"),
        "errors": None,
        "ex_cap": "Component-wise minimum",
        "ex": 'io::print(toString(vector::min(vector::Float2[2.0,3.0], vector::Float2[4.0,1.0])))',
        "see": "max, abs, scale",
    },
    "max": {
        "sig": ["vector::max(a AS T_N, b AS T_N) AS T_N"],
        "one": "component-wise maximum",
        "desc": "Returns the component-wise maximum, each component math::max(a.i, b.i).",
        "params": [
            ("a", "T_N", "the first vector"),
            ("b", "T_N", "the second vector, same type as a"),
        ],
        "ret": ("T_N", "the component-wise maximum"),
        "errors": None,
        "ex_cap": "Component-wise maximum",
        "ex": 'io::print(toString(vector::max(vector::Float2[2.0,3.0], vector::Float2[4.0,1.0])))',
        "see": "min, abs, scale",
    },
    "abs": {
        "sig": ["vector::abs(v AS T_N) AS T_N"],
        "one": "component-wise absolute value",
        "desc": (
            "Returns the component-wise absolute value, each component math::abs(v.i).\n"
            "  Integer or Fixed abs of the minimum representable value traps\n"
            "  ErrOverflow, exactly as the scalar math::abs does."
        ),
        "params": [("v", "T_N", "the vector")],
        "ret": ("T_N", "the component-wise absolute value"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Component-wise absolute value",
        "ex": 'io::print(toString(vector::abs(vector::Float2[-2.0, 3.0])))',
        "see": "min, max, scale",
    },
    "perpendicular": {
        "sig": ["vector::perpendicular(v AS T2) AS T2"],
        "one": "2D left perpendicular (-y, x)",
        "desc": (
            "2D only. Returns the left perpendicular (-v.y, v.x), a 90-degree\n"
            "  counterclockwise rotation. This is the named form of the unary 2D\n"
            "  cross and shares its implementation. Exact for every element type."
        ),
        "params": [("v", "T2", "a 2D vector (Float2, Fixed2, or Integer2)")],
        "ret": ("T2", "the left perpendicular"),
        "errors": None,
        "ex_cap": "Perpendicular of the +x axis",
        "ex": 'io::print(toString(vector::perpendicular(vector::Float2[1.0, 0.0])))',
        "see": "cross, rotate_2d",
    },
    "rotate_2d": {
        "sig": ["vector::rotate_2d(v AS T2, angle AS Float) AS T2"],
        "one": "rotate a 2D vector counterclockwise by an angle",
        "desc": (
            "2D only. Rotates v counterclockwise by angle radians:\n"
            "  (v.x*cos - v.y*sin, v.x*sin + v.y*cos). angle is a Float for every\n"
            "  element type; Fixed and Integer convert it with toFixed and use the\n"
            "  deterministic Q32.32 sin/cos, with Integer rounding each component half\n"
            "  away from zero. Float uses the in-tree Float sin/cos."
        ),
        "params": [
            ("v", "T2", "a 2D vector (Float2, Fixed2, or Integer2)"),
            ("angle", "Float", "rotation angle in radians, counterclockwise"),
        ],
        "ret": ("T2", "the rotated vector"),
        "errors": ERR_OVERFLOW,
        "ex_cap": "Rotate the +x axis by a right angle",
        "ex": 'io::print(toString(vector::rotate_2d(vector::Float2[1.0, 0.0], math::pi2)))',
        "see": "perpendicular, angle",
    },
}


def page(name, meta):
    lines = []
    lines.append("NAME")
    lines.append(f"  {name} - {meta['one']}")
    lines.append("")
    lines.append("SYNOPSIS")
    for s in meta["sig"]:
        lines.append(f"  {s}")
    lines.append("")
    lines.append("PACKAGE")
    lines.append("  vector")
    lines.append("")
    lines.append("IMPORTS")
    lines.append("  IMPORT vector")
    lines.append("")
    lines.append("DESCRIPTION")
    lines.append("  " + meta["desc"])
    lines.append("")
    if len(meta["sig"]) > 1:
        lines.append("OVERLOADS")
        for s in meta["sig"]:
            lines.append(f"  {s}")
        lines.append("    Resolved by argument record type and arity; T is the element type")
        lines.append("    (Float, Fixed, or Integer) and N the dimension (2, 3, or 4).")
        lines.append("")
    lines.append("PARAMETERS")
    for pname, ptype, meaning in meta["params"]:
        lines.append(f"  {pname} AS {ptype}")
        lines.append(f"    {meaning}")
        lines.append("")
    lines.append("RETURN VALUE")
    rt, rmean = meta["ret"]
    lines.append(f"  AS {rt}")
    lines.append(f"    {rmean}")
    lines.append("")
    lines.append("ERRORS")
    if meta["errors"]:
        lines.append(meta["errors"].rstrip("\n"))
    else:
        lines.append("  None.")
    lines.append("")
    lines.append("TYPE CHECKING")
    lines.append("  Overloaded over the nine vector record types; the two vectors of a")
    lines.append("  binary call must be the same type. T is the element type and T_N the")
    lines.append("  N-dimensional vector. No mixed-type or cross-dimension overload exists.")
    lines.append("")
    lines.append("EXAMPLES")
    lines.append(f"  {meta['ex_cap']}:")
    lines.append("")
    lines.append("    IMPORT vector")
    lines.append("    IMPORT io")
    if "math::" in meta["ex"]:
        lines.append("    IMPORT math")
    lines.append("    FUNC main AS Integer")
    for exline in meta["ex"].split("\n"):
        lines.append(f"      {exline}")
    lines.append("      RETURN 0")
    lines.append("    END FUNC")
    lines.append("")
    lines.append("SEE ALSO")
    lines.append(f"  {meta['see']}")
    return "\n".join(lines) + "\n"


def main():
    for name, meta in FUNCS.items():
        (OUT / f"{name}.txt").write_text(page(name, meta))
    print(f"wrote {len(FUNCS)} vector man pages")


if __name__ == "__main__":
    main()
