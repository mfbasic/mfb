I have enough evidence. Writing up the findings now.

### 1. Errors tables contradict the prose on seven pages: ErrOverflow is raised but not declared
UNIT:      man-pkg:vector
PAGE:      dot, scale, cross, distance, length, perpendicular, reflect
CATEGORY:  consistency
CLAIM:     `dot` prose: "will exceed the range and fail with ErrOverflow"; `scale`: "a product that leaves the range of the element type fails with ErrOverflow"; `distance`: "It is not, however, error-free: the squaring step ... can overflow"; `perpendicular`: "fails with ErrOverflow". Yet none of these seven pages renders an Errors section at all, which by the renderer's rule means "raises no errors". Meanwhile `min`/`max` say "one of only two functions in this package (with vector::max) that raise no errors whatsoever".
VERDICT:   inconsistent
EVIDENCE:  `grep -n '&\["ErrOverflow"\]' src/codegen/builtins/vector/func_abs.rs` → line 167 (abs declares it); the same grep over func_dot.rs, func_scale.rs, func_cross.rs, func_distance.rs, func_length.rs, func_perpendicular.rs, func_reflect.rs → no hits. Runtime probes (built via `mfb init probe` + `mfb build -q probe`, scratch dir /tmp/plan-125-scratch/B-recheck/man-pkg-vector): `vector::dot(Integer2[4000000000,4000000000], same)`, `vector::scale(Integer2[4000000000,1], same)`, `vector::cross(Integer3[4e9,0,0], Integer3[0,4e9,0])`, `vector::distance(Integer2[4e9,0], Integer2[-4e9,0])`, `vector::length(Integer2[4e9,4e9])`, `vector::reflect(Integer2[4e9,4e9], same)` and `vector::perpendicular(Integer2[0, INT_MIN])` all terminate with `Error: 7-705-0010 Arithmetic overflow`. The prose is right; the derived table is wrong because the descriptors omit the declaration.
SUGGESTED: This is a descriptor bug, not a prose fix: file a bug (`bugs/bug-NNNN-*.md` + `bugs/status.md` row) that the Fixed/Integer overloads of dot, scale, cross, distance, length, perpendicular and reflect do not declare ErrOverflow, so `mfb man` renders no Errors table for them. Keep the prose as the intended contract. Until fixed, the min/max sentence "one of only two functions ... that raise no errors" is the only honest statement on the unit and should stay.

### 2. No page answers "how do I add, subtract, or negate two vectors?"
UNIT:      man-pkg:vector
PAGE:      package-wide (overview)
CATEGORY:  coverage
CLAIM:     The overview lists "geometry, utility, and 2D functions ... (length, normalize, ... rotate_2d)". Nothing on the unit says whether vector records support `+`, `-`, unary `-` or `*`, and no function provides addition, subtraction, or negation.
VERDICT:   missing
EVIDENCE:  Probe `io::print(toString(a + b))` with two `vector::Float2` → `error[2-203-0001 TYPE_BINARY_OPERATOR_MISMATCH]: binary operator operands have incompatible types`; `-a` and `a * 2.0` fail the same way. The only hints on the unit are indirect: `scale` says "this package provides no vector-times-scalar function" and `perpendicular` says "Two applications are therefore a cheap exact negation". A developer coming from any other vector library arrives with "a + b" as their first line and the unit never tells them it will not compile or what to write instead.
SUGGESTED: Add one paragraph to the overview: "Vector records have no arithmetic operators: `a + b`, `a - b`, `-a` and `a * 2.0` are compile-time type errors. Add, subtract or negate by building a new record from the components, as in `vector::Float2[a.x + b.x, a.y + b.y]`; multiply by a scalar with vector::scale and a vector whose components are all that scalar. Records compare with `=` and print with toString as `(1.00, 2.00)`." (Equality and toString verified by probe: `a = b` printed TRUE, `toString(vector::zeroInteger3)` printed `(0, 0, 0)`.)

### 3. min/max mention a Money element type the package does not have
UNIT:      man-pkg:vector
PAGE:      min, max
CATEGORY:  consistency
CLAIM:     "Fixed, Integer, and Money comparisons are a signed 64-bit compare and select over the underlying representation."
VERDICT:   wrong
EVIDENCE:  Overview and `mfb man vector types` list exactly nine record types over Float, Fixed and Integer; `grep -c Money src/codegen/builtins/vector/mod.rs` → 0. The only Money mentions in the package are these two sentences (`grep -n Money src/codegen/builtins/vector/func_min.rs func_max.rs` → line 27 of each), copied from the scalar math::min/max text. "signed 64-bit compare over the underlying representation" is also implementation vocabulary with no developer meaning.
SUGGESTED: "Float comparisons take the larger of the two directly; Fixed and Integer comparisons are exact." Drop Money and the representation clause on both pages.

### 4. cross and perpendicular give opposite guidance on which to use in 2D
UNIT:      man-pkg:vector
PAGE:      cross, perpendicular
CATEGORY:  consistency
CLAIM:     `cross`: "Use whichever name reads better where you are: cross when the surrounding code is doing cross products across dimensions, perpendicular when the point is the quarter turn itself." `perpendicular`: "They are nevertheless two separate functions, and neither is a shorthand for the other: prefer vector::perpendicular when you mean a quarter turn, and vector::cross when you mean the generalized product."
VERDICT:   misleading
EVIDENCE:  Both read from the rendered pages above. The two pages agree the values are identical, but one says "whichever reads better" while the other insists "neither is a shorthand for the other", which a reader will take as a warning that they differ in some way. Both have the same error behaviour at runtime (the perpendicular probe raised ErrOverflow on INT_MIN; the cross 2D form is the same expression).
SUGGESTED: Use one sentence on both pages: "The 2D unary cross and perpendicular compute the same value with the same error behaviour; prefer perpendicular when you mean a quarter turn and cross when the surrounding code is doing cross products across dimensions."

### 5. The overview does not say what prints or compares, nor how a record is built
UNIT:      man-pkg:vector
PAGE:      package-wide (overview)
CATEGORY:  discoverability
CLAIM:     Every example on the unit uses `vector::Float2[0.0 - 2.0, 3.0]` construction and `toString(...)`, but the overview never introduces the bracket constructor, the `.x/.y/.z/.w` field access, or the printed form, and the types page gives only "The first component."
VERDICT:   missing
EVIDENCE:  `mfb man vector` Description (quoted above) covers types, functions, constants and rounding only. `grep -n "\[" ` on the overview text finds no constructor mention. Probe: `toString(vector::Float2[1.0, 2.0])` → `(1.00, 2.00)`, `toString(vector::zeroInteger3)` → `(0, 0, 0)`, `a = b` → TRUE. Note also that the examples write `0.0 - 2.0` for a negative literal, which a reader will copy without knowing whether `-2.0` is legal inside the brackets. UNVERIFIED whether `vector::Float2[-2.0, 3.0]` parses; I did not probe it.
SUGGESTED: Add to the overview, before the function list: "Build a vector with the record constructor, `vector::Float3[1.0, 2.0, 3.0]`, read components as `v.x`, `v.y`, `v.z`, `v.w`, compare whole vectors with `=`, and print one with toString, which gives `(1.00, 2.00, 3.00)`." Then fold finding 2 in as the next sentence.
