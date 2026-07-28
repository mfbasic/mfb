# MFBASIC `vector::` Math Package Plan

Last updated: 2026-06-28

This plan adds a `vector::` standard package of small fixed-width math vectors —
the nine value records `Float2/3/4`, `Fixed2/3/4`, `Integer2/3/4` — plus a full
set of overloaded geometry, utility, and 2D functions over them and a set of
package-level constants. A correct implementation lets a program write
`vector::normalize(vector::Float3(3.0, 0.0, 4.0))` and get back a unit-length
`Float3`, with every overload selected by argument record type, **all results
bit-deterministic across platforms — Float trig included** (plan-01-libm-kernels
moved `math::`'s Float `sin`/`cos`/`tan`/`asin`/`acos`/`atan`/`atan2` off libm
onto hand-written in-tree kernels, so there is **no** determinism caveat left;
see §2/Decision 2), and value/copy/transfer/golden semantics for existing
programs untouched.

The members fall in five groups, every one resolved by exact argument record
type/arity via the existing FUNC-overloading mechanism `[[func-sub-overloading]]`:

1. **Core geometry** — `length`, `normalize`, `distance`, `dot`, `cross`.
2. **Derived geometry** — `reflect`, `project`, `reject`, `angle`.
3. **Interpolation & magnitude** — `lerp` (now clamped), `lerp_unclamped`,
   `slerp`, `clamp_length`.
4. **Component-wise utilities** — `scale`, `min`, `max`, `abs`.
5. **2D-specific + presentation + constants** — `perpendicular`, `rotate_2d`,
   `toString`, and the package-level constants `zero`/`one`/`up`/`right`/`forward`.

The package is built as a **source companion** (`vector_package.mfb`, the
established `json`/`csv`/`datetime`/`net`/`regex` idiom) for its correctness spine,
then a final phase **accelerates the Float/Fixed overloads with inline NEON SIMD**.
It is sequenced after **plan-01-simd** for that phase only: plan-01 builds the
AArch64 vector encoder (V-register operands, `fmul.2d`/`fadd.2d`/`fsqrt`/`dup`, etc.)
the acceleration layer consumes. A `Float4` is exactly two 128-bit `.2d` lanes;
`dot`/`length`/`add`/`sub`/`lerp` are textbook no-loop-no-tail SIMD kernels, so the
payoff is clean — but **the spine does not depend on plan-01 and lands first**.

**Determinism note (decided up front, see §2 / Decision 2).** Every *algebraic*
operation this package needs — IEEE `+ − × ÷` and `sqrt` — is correctly-rounded on
AArch64: scalar `math::sqrt(Float)` lowers to the hardware `FSQRT` instruction,
`math::sqrt(Fixed)` is deterministic Q32.32, and Integer uses a deterministic
integer square root. **All algebraic overloads — including every Float overload —
are bit-identical on macOS and Linux from the phase that lands them**, and the §7
SIMD layer is required to preserve those exact bits (it is pure speed, not a
determinism fix).

**There is no longer any trig caveat.** As of plan-01-libm-kernels (which
completed plan-01-simd's work), `math::`'s *Float* `sin`/`cos`/`tan`/`asin`/`acos`/
`atan`/`atan2` no longer ride libm — they are hand-written in-tree NEON `f64`
kernels (double-double-compensated polynomials / fdlibm 4-segment `atan`),
**bit-identical on macOS / Linux-glibc / Linux-musl** and within ≤1 ULP of macOS
libm (`tan` is faithfully rounded). So the three intrinsically-transcendental
members — `angle`, `slerp`, `rotate_2d` — are now **fully deterministic across
platforms for every element type**: their Fixed/Integer overloads via `math::`'s
deterministic Q32.32 trig (raw fixed-point, not host floating point — `mfb man
math`), and their Float overloads via the deterministic in-tree Float kernels.
All 22 functions and all element types are bit-identical across targets — a clean
guarantee with no exception. (When this plan was first written, the Float trig was
still libm-backed and carried a "last-ULP may vary across platforms" caveat; that
caveat is now obsolete.)

It complements:

- `./mfb spec language builtin-functions` (the member list this package adds; canonical under `src/spec/language/**`)
- `./mfb spec memory` records/collections (vector values are ordinary value records — N contiguous 8-byte fields; this plan adds **no** new layout)
- `./mfb spec diagnostics error-codes` (`ErrInvalidArgument` `77050002` for a zero-length `normalize`; `src/spec/diagnostics/02_error-codes.md`)
- `./mfb spec architecture aarch64-instruction-set` (the NEON `CodeOp` set plan-01-simd adds and the acceleration layer in §7 reuses; correctly-rounded `FSQRT`/`FADD`/`FMUL`)
- New stdlib doc topic `src/spec/stdlib/08_vector.md` (precedents `02_datetime.md`, `06_url.md`)

## 1. Goal

Add the `vector` package, importable as `IMPORT vector`, exposing:

**Types** (qualified `vector::Name`, normalized to a bare id at parse time exactly
like `net::Url`/`http::Response` — `src/ast.rs:3092`, `:2694`):

| Type | Fields |
|---|---|
| `Float2` / `Float3` / `Float4` | `x,y[,z[,w]] AS Float` |
| `Fixed2` / `Fixed3` / `Fixed4` | `x,y[,z[,w]] AS Fixed` |
| `Integer2` / `Integer3` / `Integer4` | `x,y[,z[,w]] AS Integer` |

**Functions** (every overload resolved by exact arg record type via the existing
FUNC-overloading mechanism `[[func-sub-overloading]]`). Unless a row says
otherwise, each function has one overload per N∈{2,3,4} × T∈{Float,Fixed,Integer}
(9 overloads). "Det." = bit-deterministic across platforms for that element type
(✓ = yes, **for every element type including Float** — `math::`'s Float trig is
now in-tree and deterministic, so even the trig members carry no caveat; see
§2/Decision 2).

*Core geometry (§4.1–4.5):*

| Function | Signature pattern | Result | Det. |
|---|---|---|---|
| `length` | `(v AS T_N)` | scalar `T` | ✓ |
| `normalize` | `(v AS T_N)` | `T_N` | ✓ |
| `distance` | `(a AS T_N, b AS T_N)` | scalar `T` | ✓ |
| `dot` | `(a AS T_N, b AS T_N)` | scalar `T` | ✓ |
| `cross` | `(v AS T2)` · `(a AS T3, b AS T3)` · `(a AS T4, b AS T4, c AS T4)` | `T_N` | ✓ |

`cross` is the generalized **(n−1)-ary** cross product, so its arity is
dimension-specific: 1 vector in 2D, 2 in 3D, 3 in 4D (§4.5). It resolves by arity +
type, which the existing FUNC-overloading mechanism already handles.

*Derived geometry (§4.8–4.11):*

| Function | Signature pattern | Result | Det. |
|---|---|---|---|
| `reflect` | `(v AS T_N, n AS T_N)` | `T_N` | ✓ |
| `project` | `(a AS T_N, b AS T_N)` | `T_N` | ✓ |
| `reject` | `(a AS T_N, b AS T_N)` | `T_N` | ✓ |
| `angle` | `(a AS T_N, b AS T_N)` | scalar `T` (radians) | ✓ |

*Interpolation & magnitude (§4.6, §4.12–4.14):*

| Function | Signature pattern | Result | Det. |
|---|---|---|---|
| `lerp` | `(a AS T_N, b AS T_N, t AS Float)` — **clamps `t` to `[0,1]`** | `T_N` | ✓ |
| `lerp_unclamped` | `(a AS T_N, b AS T_N, t AS Float)` — extrapolates | `T_N` | ✓ |
| `slerp` | `(a AS T_N, b AS T_N, t AS Float)` | `T_N` | ✓ |
| `clamp_length` | `(v AS T_N, max AS T)` | `T_N` | ✓ |

*Component-wise utilities (§4.15):*

| Function | Signature pattern | Result | Det. |
|---|---|---|---|
| `scale` | `(a AS T_N, b AS T_N)` | `T_N` | ✓ |
| `min` | `(a AS T_N, b AS T_N)` | `T_N` | ✓ |
| `max` | `(a AS T_N, b AS T_N)` | `T_N` | ✓ |
| `abs` | `(v AS T_N)` | `T_N` | ✓ |

*2D-specific + presentation (§4.16–4.18):*

| Function | Signature pattern | Result | Det. |
|---|---|---|---|
| `perpendicular` | `(v AS T2)` — **2D only**, 3 overloads | `T2` | ✓ |
| `rotate_2d` | `(v AS T2, angle AS Float)` — **2D only**, 3 overloads | `T2` | ✓ |
| `toString` | `(v AS T_N)` — general-builtin override | `String` | ✓ |

**Constants** (package-level `EXPORT LET` values, type+dimension suffixed — the
`math::pi`/`math::piFixed` idiom — referenced `vector::<const><Type><N>`, e.g.
`vector::zeroFloat3`, `vector::upInteger2`, `vector::forwardFixed4`; §4.19):

| Constant | Value | Types |
|---|---|---|
| `zero<Type><N>` | all-zero | all 9 |
| `one<Type><N>` | all-one | all 9 |
| `up<Type><N>` | `+y` axis: 2D `(0,1)`, 3D `(0,1,0)`, 4D `(0,1,0,0)` | all 9 |
| `right<Type><N>` | `+x` axis: 2D `(1,0)`, 3D `(1,0,0)`, 4D `(1,0,0,0)` | all 9 |
| `forward<Type><N>` | `+z` axis: 3D `(0,0,1)`, 4D `(0,0,1,0)` | **3D/4D only** (6) — `forward` is undefined in 2D |

Concrete checkable outcome: each of the 9 types constructs and copies; each of the
~168 function overloads (the §1 tables) plus the 42 constants compiles, resolves by
type/arity, runs, and returns the value the formula in §4 specifies; **all results
(Float included, trig members included) are bit-identical on macOS and Linux**
(`math::` Float trig is now in-tree, §2/Decision 2); a zero-length `normalize` exits 255 with
`ErrInvalidArgument`; acceptance passes; every overload has
`tests/func_vector_<fn>_*_valid/**` and
`_invalid/**`.

### Non-goals (explicit constraints)

- **No new value-record layout or ABI.** Vector types are ordinary value records of
  N homogeneous 8-byte scalar fields (`mfb spec memory`). They copy by value,
  scope-drop with no heap frees (all-scalar, no owned pointers), and are trivially
  thread-sendable — no new copy/move/freeze/transfer rule. Existing programs'
  goldens are unaffected.
- **No change to scalar `math::`.** `length`/`normalize`/`distance` reuse the
  existing scalar `math::sqrt(Float)` (hardware `FSQRT`) / `math::sqrt(Fixed)`
  (Q32.32) and a deterministic integer `isqrt`; `math::` codegen and goldens are
  untouched.
- **No floating-point non-determinism, ever.** No libm (not even via the trig
  members — `math::`'s Float trig is now in-tree, see §2/Decision 2), no FMA
  contraction in the *algebraic* members, no reduction-order ambiguity: §4 fixes
  one canonical left-to-right evaluation order, and **both the source spine and
  the §7 SIMD path emit it bit-for-bit** (separate `fmul`+`fadd`, no `fmla`;
  correctly-rounded `fsqrt`/`fdiv`). Fixed stays Q32.32; Integer stays integer
  arithmetic.
- **No second architecture.** AArch64 only. The §7 SIMD layer emits only the
  `CodeOp`s plan-01-simd already added; it introduces no new instruction.
- **No vector-of-vector SIMD, no swizzles, no operator overloading** (`a + b` on
  vectors), no `Byte`/other element types, no dynamic-length vectors. Only the 9
  listed types, the §1 function set, and the §1 constants.

## 2. Current State

**Source-package idiom.** `json`/`csv`/`datetime`/`net`/`regex`/`http` are each a
`*_package.mfb` file parsed by `<pkg>::source_file()` and appended to the project
AST by `<pkg>::augmented_project(ast)` when `uses_package(ast)` sees the import
(`src/builtins/net.rs:286`–`312`; wired in `src/resolver.rs:66`–`80`). The companion
file `EXPORT TYPE`s become real program types (`datetime_package.mfb:19` `Instant`,
`json_package.mfb:4` `JsonNull`, …) and its `FUNC`s become callable members.

**Two type-reference patterns.** (a) *Bare* builtin types — `datetime::Instant` is
registered bare in `datetime::is_builtin_type` and referenced unqualified
(`datetime.rs:72`). (b) *Qualified* builtin types — `net::Url`/`http::Response` are
registered in `<pkg>::is_builtin_type` and resolved through
`builtins::qualified_builtin_type(name)` (`src/builtins/mod.rs:73`), which splits
`pkg.Member`, checks `is_builtin_import(pkg)` + `is_builtin_type(Member)`, and
**normalizes the qualified name to its bare internal id at parse time** so all
downstream stages see only the bare id (`src/ast.rs:3092`, and the constructor form
`src/ast.rs:2694`). The user spelled the types `vector::FloatN` — pattern (b).

**Overload resolution by type** is already supported and documented
(`[[func-sub-overloading]]`, mfbasic.md §6/§7): overloads resolve by exact arity +
positional argument types. The §1 function set across the 9 types is ~168 plain FUNC
overloads in the companion file (plus the dimension-restricted `cross`/
`perpendicular`/`rotate_2d`/`forward` cases); no new resolution machinery is needed.

**Math is correctly-rounded and deterministic.** `math::sqrt` has scalar
`Float`→`Float` (hardware `FSQRT`) and `Fixed`→`Fixed` (deterministic Q32.32)
overloads (`builder_math.rs`); IEEE `+ − × ÷` lower to the correctly-rounded
scalar/NEON ops. There is **no** `math::sqrt(Integer)`, so Integer
`length`/`distance` need a deterministic integer square root (a small MFBASIC
Newton/bisection `isqrt`, rounding to nearest — §4.1). `^` exists (`lexer.rs:245`,
`ast.rs:2588`) but the formulas use plain multiplication (`v.x * v.x`) to avoid `pow`
semantics.

**Trig is available and deterministic for every element type.** `math::` exposes
the full `sin`/`cos`/`tan`/`asin`/`acos`/`atan`/`atan2` family over Float and Fixed
(`mfb man math`). The Fixed transcendentals use raw Q32.32 fixed-point arithmetic
rather than host floating point, and — as of plan-01-libm-kernels — the **Float**
transcendentals are likewise hand-written in-tree NEON `f64` kernels (no libm), so
**both** are deterministic and identical across macOS / Linux-glibc / Linux-musl
(Float within ≤1 ULP of macOS libm; `tan` faithfully rounded). `angle`/`slerp`/
`rotate_2d` are the only members that call these — the Fixed (and via Fixed, the
Integer) overloads use the Fixed trig, the Float overloads use the Float trig, and
**all of them are now bit-identical across targets** (no caveat). Fixed `angle`
returns radians as a Fixed; Integer `angle` rounds the Fixed-radian result. Domain
failures from `math::` trig propagate as-is (`ErrFloatDomain`/`ErrInvalidArgument`),
e.g. `acos` of an argument nudged outside `[-1,1]` by rounding — §4.11 clamps the
cosine to `[-1,1]` before `acos` to keep `angle` total.

**Package-level constants are `EXPORT LET` values.** `math::` ships its constants as
exported `LET` values type-suffixed for the Fixed variant (`math::pi` Float,
`math::piFixed` Fixed). `EXPORT LET` over a top-level binding is supported
(`src/ast/items.rs:412`, `:458`). The vector constants follow the same idiom with a
full `<Type><N>` suffix (no natural default element type across 9 records), e.g.
`EXPORT LET zeroFloat3 AS Float3 = Float3(0.0, 0.0, 0.0)`. Phase 1 verifies that a
top-level `EXPORT LET` accepts a record-constructor initializer; if it does not, the
fallback is zero-arg `EXPORT FUNC`s of the same names (`vector::zeroFloat3()`),
chosen because zero-arg functions cannot overload by return type and so must carry
the type in the name regardless (Decision 9).

**Override hook** (`general_override_target`, `mod.rs:65`; `internal_override_*`,
`[[overridable-builtins-returntype-overloads]]`) is how a package overrides a
general builtin (`toString(net::Url)`); the §7 acceleration layer reuses this same
"internal helper replaces a source FUNC at lowering" pattern to swap a Float/Fixed
overload for an intrinsic without changing its surface.

**plan-01-simd (assumed complete first).** Adds the AArch64 NEON encoder: V-register
operands with arrangement suffixes (`v0.2d`, …), and the vector `CodeOp`s
`fadd/fsub/fmul/fdiv/fsqrt/fabs/fmin/fmax`, `dup`, integer `add/sub/mul/shl/sshr`,
with `#[cfg(test)]` encoder words. §7 consumes exactly these; it adds none.

## 3. Design Overview

Three layers, built spine-first so each phase is independently landable and the
SIMD-codegen risk lands last behind tests:

1. **Type surface (Phase 1).** Register the 9 qualified builtin record types
   (`vector::is_builtin_type` + `qualified_builtin_type` wiring) and declare them as
   `EXPORT TYPE`s in `vector_package.mfb`. Construction + copy + field access work;
   no functions yet.
2. **Source-package functions (Phases 2–6).** All ~168 function overloads (plus
   the 42 constants and the `toString` overrides) implemented in MFBASIC in the
   companion file: pure field arithmetic in the canonical order of §4 + `math::sqrt`
   (Float/Fixed) / `isqrt` (Integer), and `math::` trig for the three trig members.
   Correct **and bit-deterministic on day one for every element type** (algebraic
   overloads Float-included; the three trig members deterministic for every element
   type, Float included, now that `math::` Float trig is in-tree — §2/Decision 2).
   **No dependency on plan-01.**
3. **SIMD acceleration (Phase 7).** Reimplement the SIMD-friendly Float (and
   integer-NEON-friendly Fixed) algebraic overloads as intrinsics lowered to NEON via
   plan-01's encoder, emitting the **same canonical evaluation order** so results are
   **bit-identical** to the spine — i.e. goldens do **not** change. The trig members
   stay on the source spine. Pure speed; the surface and tests from earlier phases
   are unchanged.

Correctness risk concentrates in (a) the geometric formulas and their
Fixed/Integer rounding (Phases 2–6), (b) the degenerate cases the user's signatures
force — Integer `normalize`/`project`/`angle`/`slerp`/`rotate_2d`, the
dimension-specific arity of `cross` (§4.5), and the division/zero-vector error paths
in `project`/`reject`/`angle`/`clamp_length` — (c) routing the three trig members to
deterministic Fixed trig while keeping `acos` total via cosine clamping (§4.11), and
(d) the NEON kernels reproducing the canonical order bit-for-bit and the
result-record construction (Phase 7).

## 4. Detailed Design — semantics (Phases 2–6)

All formulas operate component-wise in a **fixed canonical left-to-right order** (the
order the user wrote, e.g. `v.x*v.x + v.y*v.y + v.z*v.z`); the §7 SIMD path emits the
same order so the two paths agree to the bit. `T` is the element type. Fixed uses
Q32.32 fixed arithmetic; Integer uses integer arithmetic with the **one rounding
rule** stated next; Float uses IEEE `Float` arithmetic.

**Integer rounding rule (decided).** Every Integer result that comes from a
real-valued computation (`length`, `distance`, `normalize` components, `lerp`)
**rounds half away from zero**, matching `math::round`. `dot` and `cross` are exact
integer arithmetic (no rounding). This is more correct than truncation (it preserves
direction/magnitude better) and is one consistent rule across the package.

### 4.1 `length(v) AS T`
`sqrt(sum of squared components)`, summed left-to-right:
- `Float`: `math::sqrt(v.x*v.x + v.y*v.y [+ v.z*v.z [+ v.w*v.w]])` — hardware
  `FSQRT`, deterministic.
- `Fixed`: same with Fixed multiply/add and `math::sqrt(Fixed)` (deterministic).
- `Integer`: a deterministic **rounding** integer square root of the squared sum
  (round half away from zero), computed in integer arithmetic — no Float round-trip,
  so it is exact even for large sums. (Overflow of the squared sum is the standard
  Integer-overflow trap; no new path.)

### 4.2 `normalize(v) AS T_N`
Returns a same-direction vector of length 1:
- `len = length(v)`; if `len == 0` → **`FAIL error(77050002, "vector::normalize of a
  zero-length vector")`** (`ErrInvalidArgument`) — a loud trap, not a silent zero
  vector, matching plan-01's domain-error stance (negative `sqrt`, etc.).
- `Float`/`Fixed`: each component `c / len` (correctly-rounded `fdiv` / Q32.32
  divide).
- `Integer`: each component is `c / len` rounded half away from zero. Intentionally
  lossy — most Integer unit vectors land in `{-1,0,1}` — but it is exactly the
  requested signature and the rounding keeps the result as direction-faithful as
  integers allow. Documented as a known quirk (§4.7 / DOC block).

### 4.3 `distance(a, b) AS T`
`length(a - b)` — `length` of the component-wise difference, same per-type rules and
the same Integer rounding-`isqrt` determinism.

### 4.4 `dot(a, b) AS T`
`a.x*b.x + a.y*b.y [+ a.z*b.z [+ a.w*b.w]]`, summed left-to-right. Float/Fixed/Integer
per their arithmetic; exact for Integer (no rounding). No error path (Integer
overflow traps normally).

### 4.5 `cross` — the generalized (n−1)-ary cross product
The cross product in `n` dimensions takes `n−1` vectors and returns the vector
perpendicular to all of them (the formal determinant of the `n×n` matrix whose first
row is the basis vectors and whose remaining rows are the operands). The signatures
follow that arity exactly — this is mathematically standard, so no convention is
invented:

- **`*2` (unary), `cross(v) AS T2`:** the left perpendicular `(-v.y, v.x)` (the 90°
  counterclockwise rotation; the `n−1 = 1` case).
- **`*3` (binary), `cross(a, b) AS T3`:** `(a.y*b.z - a.z*b.y, a.z*b.x - a.x*b.z,
  a.x*b.y - a.y*b.x)`.
- **`*4` (ternary), `cross(a, b, c) AS T4`:** each component is a signed `3×3`
  cofactor of the `3×4` matrix `[a; b; c]` (cofactor expansion along the basis row),
  with `x,y,z,w` ↔ columns `1,2,3,4`:
  - `r.x = +(a.y*(b.z*c.w - b.w*c.z) - a.z*(b.y*c.w - b.w*c.y) + a.w*(b.y*c.z - b.z*c.y))`
  - `r.y = -(a.x*(b.z*c.w - b.w*c.z) - a.z*(b.x*c.w - b.w*c.x) + a.w*(b.x*c.z - b.z*c.x))`
  - `r.z = +(a.x*(b.y*c.w - b.w*c.y) - a.y*(b.x*c.w - b.w*c.x) + a.w*(b.x*c.y - b.y*c.x))`
  - `r.w = -(a.x*(b.y*c.z - b.z*c.y) - a.y*(b.x*c.z - b.z*c.x) + a.z*(b.x*c.y - b.y*c.x))`

All forms are evaluated in the §4 canonical order. Fixed/Integer use their
arithmetic (exact; Integer overflow traps normally; no rounding).

### 4.6 `lerp(a, b, t) AS T_N` / `lerp_unclamped(a, b, t) AS T_N`  (note: `t AS Float` for **all** element types)
Component-wise `a + (b - a) * t`. The two functions differ **only** in whether `t`
is clamped first (Decision 7):
- **`lerp`** clamps `t` to `[0,1]` before interpolating (`t = math::clamp(t, 0.0,
  1.0)`), so the result always lies on the segment `[a,b]` — the engine-conventional
  "clamped lerp".
- **`lerp_unclamped`** uses `t` verbatim, extrapolating outside `[0,1]` (the prior
  behavior of this plan's single `lerp`).

Per element type, identically for both:
- `Float`: direct IEEE arithmetic.
- `Fixed`: `t` (Float) is first converted to Fixed via the deterministic `toFixed`,
  then Fixed arithmetic — fully deterministic. The clamp (in `lerp`) is applied in
  Float before conversion.
- `Integer`: compute `a + (b - a) * t` in `Float`, then round half away from zero to
  `Integer` per component (the §4 rounding rule). Float `a`/`b` come from exact
  Integer→Float widening; the single rounding is at the final store.

### 4.7 Companion-file gotchas (from prior source packages)
Fold in the known idioms (`[[regex-package-impl]]`, `[[csv-package-impl]]`,
`[[net-package-impl]]`, `[[func-sub-overloading]]`): reserved words (`next`, etc.),
≤ 8 params per FUNC (max here is 3 — fine), no field assignment after construction
(build each result record in one constructor call), cross-file refs need `EXPORT`,
`\\` escaping in any string literal, and **defaults do not combine with
overloading** — every overload is a distinct full signature.

### 4.8 `reflect(v, n) AS T_N`
Reflects `v` about the hyperplane with normal `n`: `v - 2*dot(v,n)*n`, component-wise
in the §4 order. `n` is taken as given (the caller supplies a unit normal; `reflect`
does **not** normalize it — matching every engine's `reflect`). Pure
multiply/subtract, so **exact for all three element types** (no division, no
rounding, no error path — Integer is exact too).

### 4.9 `project(a, b) AS T_N`
Vector projection of `a` onto `b`: `(dot(a,b) / dot(b,b)) * b`, component-wise.
- `b` must be non-zero: if `dot(b,b) == 0` → **`FAIL error(77050002, "vector::project
  onto a zero-length vector")`** (`ErrInvalidArgument`), same loud-trap stance as
  zero-length `normalize`.
- `Float`/`Fixed`: correctly-rounded `fdiv` / Q32.32 divide for the scalar
  `dot(a,b)/dot(b,b)`, then component multiply.
- `Integer`: the scalar quotient and each component are rounded half away from zero
  (§4 rule); intentionally lossy, documented like Integer `normalize`.

### 4.10 `reject(a, b) AS T_N`
The component of `a` orthogonal to `b`: `a - project(a, b)`, component-wise. Same
zero-`b` error path and same per-type rounding as `project` (§4.9). For Integer,
`reject` is computed as `a - project(a,b)` using the rounded projection (one rounding
per component at the projection store, then exact subtraction) so `project + reject`
round-trips as closely as integers allow.

### 4.11 `angle(a, b) AS T` (radians)
The unsigned angle between `a` and `b` in radians:
`acos( clamp( dot(a,b) / (length(a)*length(b)), -1, 1 ) )`.
- The cosine is **clamped to `[-1,1]`** before `acos` so floating/Fixed rounding can
  never push the argument out of `acos`'s domain — `angle` is total for any two
  non-zero vectors.
- Either input zero-length → **`FAIL error(77050002, "vector::angle with a
  zero-length vector")`** (`ErrInvalidArgument`).
- `Fixed`: `math::acos(Fixed)` (deterministic Q32.32) — bit-identical across targets.
- `Integer`: compute the cosine in Fixed (exact integer dot / Fixed lengths),
  `math::acos(Fixed)`, then round the radian result half away from zero to Integer —
  degenerate (radians 0–3) but exactly the requested signature; documented quirk.
- `Float`: `math::acos(Float)` — the in-tree NEON `acos` kernel (deterministic
  across targets; no libm).

### 4.12 `slerp(a, b, t) AS T_N`  (`t AS Float`)
Spherical linear interpolation along the great-circle arc from `a` to `b`. Let
`omega = angle(a,b)` (§4.11) and `s = math::sin(omega)`:
`result = (sin((1-t)*omega)/s) * a + (sin(t*omega)/s) * b`, component-wise, **no
clamping of `t`** (matches `lerp_unclamped`'s extrapolation; if a clamped variant is
wanted the caller clamps `t`).
- **Degenerate fallback:** when `s` is ~0 (vectors near-parallel or near-antiparallel,
  `omega → 0` or `→ π`) the formula is numerically unstable; fall back to
  `lerp_unclamped(a, b, t)` for `omega` below a small Fixed/Float epsilon. This is the
  standard slerp guard and keeps the function total.
- Either input zero-length → the same `ErrInvalidArgument` as `angle`.
- `Fixed`/`Integer` use deterministic Fixed trig (Integer rounds at the final store);
  `Float` uses the in-tree NEON Float trig (also deterministic — no libm caveat).
- **Note** `slerp` interpolates *direction*; it does not preserve magnitude unless
  `length(a) == length(b)` (standard). Documented in the DOC block.

### 4.13 `clamp_length(v, max) AS T_N`  (`max AS T`, scalar)
Caps the magnitude of `v` at `max`, leaving direction unchanged:
- `max < 0` → **`FAIL error(77050002, "vector::clamp_length with negative max")`**
  (`ErrInvalidArgument`).
- `len = length(v)`; if `len <= max` (or `len == 0`) return `v` unchanged (a
  zero-length `v` is returned as-is — **no** divide-by-zero, since clamping a zero
  vector to any non-negative `max` is the zero vector).
- otherwise scale each component by `max/len` (correctly-rounded `fdiv` / Q32.32 /
  rounded-Integer per §4).

### 4.14 (reserved — see §4.12/§4.13)

### 4.15 Component-wise utilities — `scale`, `min`, `max`, `abs`
All component-wise, in the §4 order, **exact for every element type** (no division,
no roots, no rounding):
- `scale(a, b)` — `(a.x*b.x, a.y*b.y, …)` (the Hadamard product; complements the
  scalar-times-vector that `normalize`/`lerp` already do internally).
- `min(a, b)` / `max(a, b)` — per-component `math::min` / `math::max`.
- `abs(v)` — per-component `math::abs`; Integer/Fixed `abs` of the minimum
  representable value traps `ErrOverflow` exactly as scalar `math::abs` does (no new
  path). These names are package-qualified (`vector::min`, `vector::abs`, …) and do
  not collide with scalar `math::`/general builtins — overload resolution is by the
  vector record argument type.

### 4.16 `perpendicular(v) AS T2`  (2D only)
Returns the left perpendicular `(-v.y, v.x)` (90° counterclockwise). This is exactly
the unary 2D `cross(v)` (§4.5) under a name games code expects; both are kept (the
user listed `perpendicular` explicitly) and share one implementation. Three overloads
(`Float2`/`Fixed2`/`Integer2`); exact for all.

### 4.17 `rotate_2d(v, angle) AS T2`  (2D only, `angle AS Float` radians)
Rotates `v` counterclockwise by `angle`:
`(v.x*cos - v.y*sin, v.x*sin + v.y*cos)` where `cos = cos(angle)`, `sin =
sin(angle)`, evaluated in the §4 order.
- `Fixed2`: `angle` → Fixed (`toFixed`), deterministic Fixed `sin`/`cos` —
  bit-identical.
- `Integer2`: compute in Fixed, round each component half away from zero —
  degenerate but per the requested signature.
- `Float2`: `math::sin`/`cos(Float)` — the in-tree NEON Float kernels
  (deterministic across targets; no libm caveat).

### 4.18 `toString(v) AS String`
Overrides the general `toString` builtin for each of the 9 vector types via the
`general_override_target` hook (`src/builtins/mod.rs:65`; precedent
`toString(net::Url)`), rendering `"(x, y, z)"` with the element type's own scalar
`toString` per component and `", "` separators — e.g. `Float3(3.0,0.0,4.0)` →
`"(3, 0, 4)"`, `Integer2(1,2)` → `"(1, 2)"`. The user's `to_string`/`__tostring`
spelling maps to MFBASIC's single `toString` member; one override per type covers
both `toString(v)` and any interpolation that calls it.

### 4.19 Constants
The 42 package-level `EXPORT LET` values of §1's constants table, each a record
literal in the §1 axis convention (`+x` right, `+y` up, `+z` forward; `forward` only
for 3D/4D). Naming `<const><Type><N>` (`zeroFloat3`, `oneInteger2`, `upFixed4`,
`rightInteger3`, `forwardFloat4`). Mechanism and fallback per §2 (`EXPORT LET` with a
record-constructor initializer; zero-arg `EXPORT FUNC` fallback if top-level record
initializers are unsupported). No element-type/dimension is omitted except `forward`
in 2D (mathematically undefined there).

## 5. Type registration (Phase 1, Rust)

Mirror `net::Url` for all nine types in a new `src/builtins/vector.rs`:
- `pub(crate) const FLOAT2_TYPE: &str = "Float2";` … through `INTEGER4_TYPE`.
- `is_builtin_type(name)` matches the nine ids.
- `builtin_type_fields(name)` returns the field list for each
  (`Float2 → &[("x","Float"),("y","Float")]`, …) so the resolver knows the shape.
- `is_builtin_import("vector")` registered alongside the other packages.
- `source_file()` / `uses_package()` / `augmented_project()` mirroring
  `net.rs:286`–`312`; wire `vector::augmented_project` into the chain in
  `src/resolver.rs` (order-independent — `vector` imports only `math`, which is
  intrinsic, so place it before `http`/`net` to keep the math-only packages grouped).
- Confirm the bare ids `Float2..Integer4` collide with **no** existing builtin type
  or reserved word, and that the nine new builtin record type IDs use the high
  reserved range (the `[[term-module-progress]]` `FIRST_TABLE_TYPE_ID` lesson) so
  they never clash with user-type numbering.

No `resolve_call`/`call_return_type_name` entries are needed for the ordinary
functions: they live in the companion file and resolve through ordinary FUNC
overloading, not the `math::`/`net::`-style intrinsic resolver.

**`toString` override (§4.18).** Add nine `general_override_target("toString", t)`
arms (`src/builtins/mod.rs:65`) — one per vector type id — each routing to a
companion renderer FUNC `__vector_toString_<type>` (e.g. `__vector_toString_float3`),
mirroring the `toString(net::Url) → __net_urlToString` precedent. The renderers are
plain companion FUNCs; only the override-table wiring is Rust.

**Constants (§4.19).** Declared entirely in the companion file as `EXPORT LET`
values; no Rust registration beyond the type wiring already above. They resolve as
qualified package values (`vector::zeroFloat3`) the same way `math::pi` does.

## 6. Layout / ABI Impact

- **No collection or record layout change.** Vector values are standard value
  records (`mfb spec memory`): N contiguous 8-byte fields, copied by value,
  scope-dropped with no heap frees, thread-sendable as scalar aggregates. Copy/
  transfer/golden output for existing programs is unaffected.
- **`mfb spec language builtin-functions`** gains the full `vector::` member list
  (the ~22 functions, the 42 constants, the `toString` overrides) and the nine type
  signatures (additive).
- **New stdlib doc** `src/spec/stdlib/08_vector.md` (+ `spec.md` index entry).
- **No new error code.** Every error path — zero-length `normalize`, zero-`b`
  `project`/`reject`, zero-input `angle`/`slerp`, negative-`max` `clamp_length` —
  reuses `ErrInvalidArgument` `77050002`; the `abs`/`math::`-overflow paths reuse the
  existing `ErrOverflow`; trig domain failures reuse `math::`'s existing
  `ErrFloatDomain`/`ErrInvalidArgument`. No `errorCode::` registry / `build.rs`
  change.
- **§7 only:** `mfb spec architecture aarch64-instruction-set` already gains its
  vector ops from plan-01; this package adds none — it only emits them.

## 7. SIMD acceleration (Phase 7, depends on plan-01-simd)

Replace the hot Float (and integer-NEON-friendly Fixed) overloads' source FUNCs with
intrinsics lowered in a new `src/target/shared/code/builder_vector.rs`, reusing
plan-01's encoder. Mechanism: register each accelerated overload via the override
hook (`general_override_target` precedent, `mod.rs:65`) so a resolved
`vector::dot(Float3,Float3)` routes to `__vector_dot_f3` intrinsic lowering instead
of the companion FUNC; surface, overload resolution, and tests are unchanged.

**Bit-identity is a hard requirement, not a goal.** The intrinsics must emit the §4
canonical order so the golden `.run` files from Phases 2–6 still match after Phase 7
— this phase regenerates **nothing**. Concretely:
- A `FloatN` value's N fields are contiguous 8-byte doubles → load as one (`Float2`)
  or two (`Float3/4`) `.2d` lanes (`Float3`'s 4th lane zeroed and excluded).
- `dot`/`length`/`distance`: `fmul.2d` for the products in parallel, then **sum the
  lanes in declared left-to-right order with scalar `fadd`** (no pairwise `faddp`,
  whose reassociation would change the last bit), then correctly-rounded `fsqrt` for
  length/distance. The parallel loads+multiplies are the win; the ordered reduce
  keeps the bits.
- `lerp`: `fsub.2d`, `dup` `t` to both lanes, then **separate `fmul.2d` + `fadd.2d`
  — never `fmla.2d`** (FMA's single rounding differs from the spine's mul-then-add).
- `normalize`: `length` reduce → reciprocal via correctly-rounded `fdiv` → `fmul.2d`,
  with the zero-length guard preserved (compare `len` to 0, branch to the same
  `ErrInvalidArgument` return as the spine).
- Result `FloatN`/`FixedN` record is constructed by `str q`/`str d` of the lanes
  into the freshly allocated record, exactly as a record literal is built.

Beyond the original `dot`/`length`/`distance`/`lerp`/`normalize`, the additional
**algebraic** members are SIMD-eligible on the same encoder and same canonical order:
- `scale` → `fmul.2d`; `min`/`max` → `fmin.2d`/`fmax.2d`; `abs` → `fabs.2d`;
  `reflect` → `dot` reduce + `dup` + `fmul.2d`/`fsub.2d`; `lerp_unclamped` is the
  pre-clamp `lerp` kernel and `lerp` is that kernel after a scalar `fmax`/`fmin` clamp
  of `t`; `project`/`reject`/`clamp_length` reuse the `dot`/`length` reduce + a
  scalar `fdiv` + `fmul.2d` (`reject` then `fsub.2d`). All emit separate `fmul`+`fadd`
  (no `fmla`) and ordered scalar reduces (no `faddp`), so goldens are unchanged.

Scope guard: if plan-01-simd's encoder lacks any needed op, the gap is plan-01's to
fill; this phase adds **no** `CodeOp`. **Excluded from SIMD and kept on the source
spine:** every Integer overload (NEON integer reduce buys little for N≤4 and risks
the rounding rule), and the three trig members `angle`/`slerp`/`rotate_2d` for **all**
element types (they bottom out in scalar `math::` trig, not vector algebra). `toString`
and the constants are not codegen targets.

## Phases

1. **Type surface + constants.** `src/builtins/vector.rs` (nine types,
   `is_builtin_type`, `builtin_type_fields`, `is_builtin_import`,
   source-file/augment wiring) + `vector_package.mfb` with the nine `EXPORT TYPE`s
   and the 42 `EXPORT LET` constants (§4.19). **Verify the `EXPORT LET`
   record-initializer mechanism here** and, if unsupported, switch to the zero-arg
   `EXPORT FUNC` fallback (Decision 9) before later phases depend on the names.
   Construction/copy/field-access + constant-read tests. Acceptance green.
2. **`dot` + `length` + `distance`** for all 9 types (companion FUNCs; rounding
   integer `isqrt` helper). Full `_valid`/`_invalid` per overload.
3. **`normalize` + `cross`** for all 9 types incl. the zero-length
   `ErrInvalidArgument` `_rt` tests, the documented Integer `normalize` quirk, and the
   dimension-specific `cross` arities (unary 2D / binary 3D / ternary 4D, §4.5).
4. **Derived geometry — `reflect` + `project` + `reject` + `angle`** for all 9 types,
   incl. the zero-`b`/zero-input `ErrInvalidArgument` `_rt` tests, the §4.11 cosine
   clamp, and the trig routing (deterministic for every element type — Fixed/Integer
   Q32.32 trig, Float in-tree NEON trig).
5. **Interpolation & magnitude — `lerp` (clamped) + `lerp_unclamped` + `slerp` +
   `clamp_length`** for all 9 types, incl. the `lerp`/`lerp_unclamped` clamp split
   (§4.6, Decision 7), the `slerp` near-parallel `lerp_unclamped` fallback (§4.12),
   the negative-`max` and zero-`v` `clamp_length` paths (§4.13), and Integer rounding.
6. **Utilities + 2D + presentation — `scale` + `min` + `max` + `abs` +
   `perpendicular` + `rotate_2d` + `toString`** (the nine `toString` overrides via the
   override hook, §4.18/§5). Spine complete, fully tested, and **bit-deterministic on
   every type without plan-01** (Float trig included — no caveat).
7. **SIMD acceleration** (§7): intrinsic Float (and integer-friendly Fixed) algebraic
   overloads on plan-01's encoder, emitting the canonical order so **all Phase 2–6
   goldens still match** (assert, don't regenerate); trig members stay on the spine;
   runtime proof that an odd-shaped `Float3` and a `Float4` give correct lanes.
   *Highest risk; lands last; requires plan-01-simd.*
8. **Docs + acceptance.** `mfb spec language builtin-functions` member list,
   `src/spec/stdlib/08_vector.md` (+ index), DOC blocks on the package, full
   `scripts/test-accept.sh` green. Remove this plan doc in the commit that lands
   Phase 8 (precedent `34e526c9`).

## Validation Plan

- **Function tests** — for every overload, `tests/func_vector_<fn>_<type>_valid/**`
  (e.g. `func_vector_length_float3_valid`, `func_vector_cross_integer3_valid`,
  `func_vector_reflect_float2_valid`, `func_vector_project_fixed3_valid`,
  `func_vector_slerp_float3_valid`) asserting the exact result of §4's formula, plus
  `_invalid/**` (wrong arity, wrong/mismatched arg types — e.g. `dot(Float2, Float3)`
  — scalar where a vector is expected; `perpendicular`/`rotate_2d` rejected for
  `*3`/`*4`; `forward` constants absent for `*2`) and `_rt` runtime-error dirs for the
  trap paths: zero-length `normalize`, zero-`b` `project`/`reject`, zero-input
  `angle`/`slerp`, negative-`max` `clamp_length` (Float/Fixed/Integer each). Include a
  Fixed and a Float test whose `.run` carries a fractional result (e.g.
  `normalize(Float3(3,0,4))` → `(0.6, 0, 0.8)`; `project(Float2(2,2), Float2(1,0))` →
  `(2, 0)`) to lock the deterministic bits.
- **Runtime proof:** a program that constructs known vectors of each element type,
  calls each function, and prints results that prove real per-component computation —
  e.g. `cross(Float2(1,0))` → `(0,1)` and the equal `perpendicular(Float2(1,0))` →
  `(0,1)`; `cross(Float3(1,0,0), Float3(0,1,0))` → `(0,0,1)`; the 4D ternary `cross`
  of three basis vectors → the fourth basis vector; `reflect(Float2(1,-1),
  Float2(0,1))` → `(1,1)`; `lerp(Integer2(0,0), Integer2(3,3), 0.5)` → `(2,2)` under
  round-half-away; `lerp(Float2(0,0), Float2(10,0), 2.0)` → `(10,0)` (clamped) vs
  `lerp_unclamped(...)` → `(20,0)`; `rotate_2d(Fixed2(1,0), math::pi2Fixed`-equivalent
  `)` → `(0,1)`; `scale`/`min`/`max`/`abs` on a known pair; reading `vector::zeroFloat3`,
  `vector::upInteger2`, `vector::forwardFixed4`; `toString(Float3(3,0,4))` →
  `"(3, 0, 4)"` — including each `cross` arity, an Integer `normalize`/`project` to
  lock the rounding quirk, and a `slerp` whose midpoint lies on the unit arc.
- **Cross-platform determinism:** the Float/Fixed/Integer `.run` goldens of **every**
  member — algebraic *and* the three trig members — are identical on macOS and Linux
  (no libm in any path: algebraic via correctly-rounded `FSQRT`/IEEE ops, trig via
  `math::`'s in-tree NEON/Q32.32 kernels). Assert all of them bit-exactly, Float
  trig included; no tolerance comparison is needed (the obsolete libm caveat is
  gone — §2/Decision 2). All bit-exact goldens must still pass **after** Phase 7 —
  the §7 bit-identity requirement.
- **Doc sync:** `mfb spec language builtin-functions`, `src/spec/stdlib/08_vector.md`
  (incl. the now-deterministic Float trig, the Integer-degeneracy quirks, and the `lerp`-vs-
  `lerp_unclamped` distinction); package man target regenerated if applicable.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`
  green after each phase that can affect AST/IR/native output. Phase 7 must **not**
  change any golden — a diff there is a bug, not a regeneration.

## Resolved Decisions

1. **Architecture — source-package spine (Phases 1–6) + SIMD intrinsic acceleration
   (Phase 7).** Lowest-risk-first and repo-idiomatic; ships correct and fully
   deterministic (Float trig included — §2/Decision 2) without plan-01; SIMD is then a pure-speed
   layer constrained to bit-identical output. *Rejected:* pure intrinsic codegen from
   day one (more risk,
   duplicates `math::sqrt`, no incremental landing) — and unnecessary, since
   determinism is already free (correctly-rounded `FSQRT` + IEEE arithmetic), so the
   only thing intrinsics add is speed, which belongs in an isolated final phase.
2. **Float determinism — guaranteed for ALL members, trig included (no caveat).**
   Every algebraic op is correctly-rounded on AArch64 (`FSQRT`, `FADD`, `FMUL`,
   `FDIV`), so all algebraic overloads (Float included) are bit-identical across
   platforms, and the §7 SIMD path is held to the same bits (no `fmla`, no `faddp`
   reassociation). The three trig members `angle`/`slerp`/`rotate_2d` are **also**
   fully deterministic: their Fixed/Integer overloads use `math::`'s deterministic
   Q32.32 trig, and — since plan-01-libm-kernels — their **Float** overloads use
   `math::`'s hand-written in-tree NEON Float trig kernels (no libm), bit-identical
   on macOS / Linux-glibc / Linux-musl and within ≤1 ULP of macOS libm (`tan`
   faithfully rounded). So this package has **no** determinism exception at all —
   all 22 functions × all 3 element types are bit-deterministic across targets.
   (This plan was originally written when Float trig still rode libm and carried a
   "last-ULP may vary across platforms" caveat scoped to these three members' Float
   overloads; that caveat is now obsolete — the implementer may assert the Float
   trig goldens bit-exactly across platforms.) *Note:* the §7 SIMD layer still must
   not introduce `fmla`/`faddp` reassociation in the algebraic members, and the
   trig members stay on the scalar `math::`-backed source spine (not SIMD), so
   their determinism comes straight from the `math::` kernels.
3. **`cross` is the generalized (n−1)-ary cross product (§4.5)** — unary in 2D (the
   perpendicular `(-y, x)`), binary in 3D (standard), ternary in 4D (perpendicular to
   three vectors, via the cofactor determinant). Its arity is dimension-specific and
   resolves by arity + type. This is the mathematically standard generalization, so
   no return-shape convention is invented. *Rejected:* forcing a uniform binary
   signature with ad-hoc 2D/4D conventions, or a compile error for non-3D.
4. **Integer results — round half away from zero** (`length`, `distance`,
   `normalize`, `lerp`), matching `math::round`; one rule across the package.
   *Rejected:* truncation toward zero — more lossy and direction-distorting for the
   already-degenerate Integer cases; rounding is the more correct choice.
5. **Zero-length `normalize` — trap `ErrInvalidArgument` `77050002`.** A loud,
   deterministic failure, matching plan-01's domain-error handling (negative `sqrt`,
   out-of-range `asin`). *Rejected:* returning the zero vector (silently wrong).
6. **Integer `normalize` kept** with the §4.4 rounding, per the requested signature,
   documented as inherently lossy. The same round-half-away-and-document treatment
   extends to every Integer overload that comes from a real-valued computation —
   `project`/`reject` (division), `angle`/`slerp`/`rotate_2d` (trig), `clamp_length`,
   `lerp`/`lerp_unclamped` — all kept per the user's "all 9 types where
   mathematically possible" request and flagged as degenerate where appropriate.
   *Rejected:* dropping the Integer overloads of the division/trig members — the user
   asked for all element types where mathematically possible, and rounded integer
   results are mathematically defined (just lossy).

7. **`lerp` is clamped; `lerp_unclamped` extrapolates.** The user explicitly wants the
   engine-conventional clamped/unclamped split, so `lerp` now clamps `t` to `[0,1]`
   (a behavior change from this plan's earlier single unclamped `lerp`) and
   `lerp_unclamped` preserves the extrapolating form. `slerp` is unclamped (matches
   `lerp_unclamped`); a caller wanting clamped slerp clamps `t`. *Rejected:* keeping
   `lerp` unclamped and adding `lerp_unclamped` as a pure alias (defeats the point of
   the distinction the user called out).

8. **Trig members route by element type.** `angle`/`slerp`/`rotate_2d` call
   deterministic `math::` Fixed trig for the Fixed and Integer overloads (Integer
   computing in Fixed then rounding) and `math::`'s in-tree NEON Float trig for the
   Float overload (Decision 2 — both are now deterministic, no libm). `angle` clamps its cosine to `[-1,1]` before `acos` so it is
   total; `slerp` falls back to `lerp_unclamped` near the degenerate parallel/
   antiparallel poles. *Rejected:* a single Float-trig implementation widened to all
   types (loses Fixed/Integer determinism); failing on the `acos` domain edge instead
   of clamping (turns rounding noise into a runtime trap).

9. **Constants are type+dimension-suffixed `EXPORT LET` package values** (`vector::
   zeroFloat3`, the `math::pi`/`piFixed` idiom). Zero-arg accessors cannot overload by
   return type, so the element type must live in the name regardless; `EXPORT LET` is
   the lighter form. Phase 1 verifies a top-level `EXPORT LET` accepts a
   record-constructor initializer; the fallback is identically-named zero-arg
   `EXPORT FUNC`s. *Rejected:* a single `zero(sampleVector)`-style typed accessor
   (awkward, needs a throwaway argument); omitting constants (the user listed them).

10. **`perpendicular` is the named 2D form of the unary `cross`** — both return
    `(-v.y, v.x)` and share one implementation; both are exposed because the user
    listed `perpendicular` explicitly and game code reaches for that name.
    *Rejected:* providing only one (breaks either the user's list or `cross`'s
    (n−1)-ary generality).

11. **Component-wise `scale`/`min`/`max`/`abs` are in scope** (this supersedes the
    earlier non-goal that excluded component-wise `min`/`max`/`abs`). They are exact
    for all element types and are the most-requested vector utilities; the names are
    package-qualified and do not collide with scalar `math::`. *Rejected:* leaving
    them out to keep the surface minimal — the user listed them and they are cheap,
    exact, and SIMD-friendly.

## Non-Goals

- No operator overloading (`a + b`, `a * s`) on vector types, no swizzles
  (`v.xy`), no matrix types or transforms. (Component-wise `scale`/`min`/`max`/`abs`
  *are* provided as named functions — Decision 11 — but not as operators.)
- No dynamic-length vectors, no `Byte`/other element types, no `Float2`↔`Float3`
  conversions.
- No signed 2D `cross` scalar, no `distance_squared`/`length_squared` fast paths, no
  `move_towards`/`smoothstep`, no quaternions — out of scope for this package.
- No x86/AVX backend (single-architecture compiler); the §7 layer is AArch64 NEON
  only and adds no instruction beyond plan-01-simd's set.
- No auto-vectorization of user record arithmetic — only the listed `vector::`
  functions are accelerated.

## Summary

The engineering substance is in the **semantics**, not new infrastructure: nine
plain value records, ~22 overloaded functions (~168 overloads), 42 constants, and
nine `toString` overrides ride entirely on existing machinery — qualified builtin
types (the `net::Url` pattern), FUNC overloading by argument type, `EXPORT LET`
package values (the `math::pi` pattern), the `toString` override hook (the
`net::Url` precedent), and the source-companion loader. Because every **algebraic**
operation the package needs is correctly-rounded on AArch64 (hardware `FSQRT`, IEEE
`+ − × ÷`, deterministic Q32.32 and integer `isqrt`), **every algebraic result —
Float included — is bit-identical across macOS and Linux from the phase that lands
it**. And the three
intrinsically-transcendental members `angle`/`slerp`/`rotate_2d` are now
deterministic too: their Fixed/Integer overloads use deterministic Q32.32 trig and
their Float overloads use `math::`'s hand-written in-tree NEON Float trig kernels
(plan-01-libm-kernels severed Float trig from libm), so **there is no determinism
caveat anywhere in this package** — all 22 functions × 3 element types are
bit-identical across targets. The spine (Phases
1–6) is correct, fully deterministic, and fully tested with
**no** dependency on plan-01-simd. The decided cases — Integer `normalize`/`project`/
`angle` degeneracy, the dimension-specific arity of `cross`, the `lerp`-vs-
`lerp_unclamped` clamp split, `perpendicular` as the named 2D `cross`, and a single
round-half-away Integer rule — are pinned in §4 / Resolved Decisions so the
implementer re-derives nothing. `cross` in particular is the mathematically standard
generalized (n−1)-ary product (unary 2D / binary 3D / ternary 4D), not an invented
convention. plan-01-simd is a dependency only for the final **acceleration** phase,
which must reproduce the canonical evaluation order bit-for-bit (no `fmla`, no `faddp`
reassociation), so it regenerates no goldens and is pure speed. Untouched throughout:
value-record layout/ABI, copy/transfer/scope-drop semantics, scalar `math::`, the
error-code registry, and the single-architecture backend shape.
