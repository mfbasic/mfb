# Vector Math Model

The `vector` package provides nine fixed-width math-vector value records and a
set of overloaded geometry, interpolation, utility, and 2D functions over them.
Like `datetime`/`net`, the behaviour is injected MFBASIC source;
only the type registration, the per-call return-type metadata, and the
public-call → internal-implementation mapping are on the compiler
side. This topic specifies the value model, the dispatch
model, the per-function formulas, the Integer rounding rule, and the determinism
guarantee — the *behaviour behind* the API, not the per-function signatures
(those are `./mfb man vector`). [[src/builtins/vector_package.mfb]] [[src/builtins/vector.rs]]

## The nine value records

Each type is an ordinary `EXPORT TYPE` value record of N homogeneous 8-byte
scalar fields — there is **no** new layout or ABI. They copy by value,
scope-drop with no heap frees (all-scalar), and are trivially thread-sendable.

| Type | Fields | Element |
|------|--------|---------|
| `Float2` / `Float3` / `Float4` | `x, y[, z[, w]]` | `Float` |
| `Fixed2` / `Fixed3` / `Fixed4` | `x, y[, z[, w]]` | `Fixed` |
| `Integer2` / `Integer3` / `Integer4` | `x, y[, z[, w]]` | `Integer` |

They are **qualified** built-in types: a program spells them `vector::Float3`,
normalized to the bare id `Float3` at parse time by `qualified_builtin_type`
(the `net::Url` pattern). Construction is positional with bracket syntax:
`vector::Float3[3.0, 0.0, 4.0]`. [[src/builtins/vector.rs:is_builtin_type]]

## Dispatch model

Every function is overloaded by **exact argument record type and arity**. Unlike
the other source packages, `vector` does not overload its internal helpers:
each (function, element-type, dimension) triple is a distinctly named companion
FUNC (`__vector_length_float3`, `__vector_cross_integer4`, …). The syntaxcheck
resolves the public return type from the argument types (`vector::resolve_call`),
and IR lowering rewrites the public call onto the type-specific internal name
from those same argument types (`vector::implementation_name`). A wrong arity,
a non-vector argument, or two vectors of different types is rejected at compile
time (`TYPE_CALL_ARITY_MISMATCH` / `TYPE_CALL_ARGUMENT_MISMATCH`). [[src/builtins/vector.rs:resolve_call]]

`toString(v)` over any of the nine types is routed by the universal-builtin
override hook to a companion renderer (`__vector_toString_<type>`), producing
`"(x, y, z)"` with each component rendered by its own scalar `toString`. [[src/builtins/vector.rs:tostring_override_target]]

## Determinism

There is **no** determinism caveat anywhere in this package — all functions over
all three element types are bit-identical across macOS / Linux-glibc /
Linux-musl. The algebraic members use only correctly-rounded operations
(hardware `FSQRT`, IEEE `+ − × ÷`); `Fixed` is deterministic Q32.32; `Integer`
uses a deterministic rounding integer square root. The three trig members
(`angle`, `slerp`, `rotate_2d`) route to `math::`'s deterministic kernels — the
Q32.32 Fixed trig for the Fixed/Integer overloads and the hand-written in-tree
NEON Float trig for the Float overloads (no libm). Evaluation is in a fixed
canonical left-to-right order (e.g. `v.x*v.x + v.y*v.y + v.z*v.z`).

## Integer rounding rule

Every Integer result derived from a real-valued computation — `length`,
`distance`, the `normalize` components, the `project`/`reject` quotient, `angle`,
`lerp`, `slerp`, `rotate_2d`, `clamp_length` — **rounds half away from zero** (matching
`math::round`), one consistent rule across the package. `dot` and `cross` are
exact integer arithmetic (no rounding). The Integer overloads of the
division/trig members are intentionally degenerate (most Integer unit vectors
land in `{-1, 0, 1}`) but mathematically defined and kept per the requested
"all element types where mathematically possible" surface.

The deterministic rounding integer square root is `__vector_isqrtRound`: it takes
the floor sqrt by Newton's method, then rounds up exactly when the remainder
exceeds the floor (the exact half `(f + 0.5)² = f² + f + 0.25` is never an
integer, so there is never a tie). [[src/builtins/vector_package.mfb:__vector_isqrtRound]]

## Function formulas

All component-wise, in the canonical order. `T` is the element type; `T_N` the
N-dimensional vector.

- **`length(v)`** — `sqrt(Σ vᵢ²)`. `Integer` rounds the integer sqrt.
- **`normalize(v)`** — `v / length(v)`. Zero length → `ErrInvalidArgument`
  (`77050002`). `Integer` rounds each component.
- **`distance(a, b)`** — `length(a − b)`.
- **`dot(a, b)`** — `Σ aᵢ·bᵢ` (exact for Integer).
- **`cross`** — the generalized **(n−1)-ary** cross product: unary in 2D (the
  left perpendicular `(−y, x)`), binary in 3D (`a × b`), ternary in 4D (the
  cofactor determinant perpendicular to three vectors). Its arity is therefore
  dimension-specific.
- **`reflect(v, n)`** — `v − 2·dot(v, n)·n` (`n` taken as given; not normalized).
- **`project(a, b)`** — `(dot(a, b) / dot(b, b))·b`. Zero `b` →
  `ErrInvalidArgument`.
- **`reject(a, b)`** — `a − project(a, b)`.
- **`angle(a, b)`** — `acos(clamp(dot(a, b) / (length(a)·length(b)), −1, 1))`
  radians. The cosine is clamped to `[−1, 1]` so `acos` is always in domain;
  either input zero-length → `ErrInvalidArgument`.
- **`lerp(a, b, t)`** — `a + (b − a)·t` with `t` clamped to `[0, 1]`.
  **`lerp_unclamped`** is the same without the clamp (extrapolates). `t` is
  `Float` for every element type.
- **`slerp(a, b, t)`** — spherical interpolation along the great-circle arc;
  falls back to `lerp_unclamped` near the degenerate parallel/antiparallel poles
  (`sin(ω) ≈ 0`). Interpolates *direction*, not magnitude. `t` unclamped.
- **`clamp_length(v, max)`** — caps `|v|` at `max` (direction unchanged). `max <
  0` → `ErrInvalidArgument`; a `v` already within `max` (or zero) is returned
  unchanged.
- **`scale(a, b)`** — component-wise (Hadamard) product.
- **`min(a, b)` / `max(a, b)` / `abs(v)`** — per-component `math::min`/`max`/`abs`
  (Integer/Fixed `abs` of the minimum value traps `ErrOverflow`, as scalar
  `math::abs` does).
- **`perpendicular(v)`** — 2D only, the left perpendicular `(−y, x)` (the named
  form of the unary 2D `cross`).
- **`rotate_2d(v, angle)`** — 2D only, counterclockwise rotation by `angle`
  radians (`Float`).

## Constants

42 package values, referenced no-paren as `vector::<base><Type><N>` (the
`math::pi` idiom): `zero`, `one`, `up` (`+y`), `right` (`+x`) for all nine types,
and `forward` (`+z`) for the 3D/4D types only (`forward` is undefined in 2D).
Each inlines a record constructor at every use site, so reading a constant copies
by value. [[src/builtins/vector.rs:constant_components]]

## See Also

* ./mfb man vector — the per-function API reference
* ./mfb spec memory heap-values — value-record layout (vectors add none)
* ./mfb man math — the deterministic scalar `sqrt`/trig the package builds on
