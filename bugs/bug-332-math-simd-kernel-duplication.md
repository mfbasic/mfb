# bug-332: numeric/SIMD math builders — duplicated array-kernel drivers, re-inlined kernel helpers, and misplaced/misattached structure

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup / duplication)

Status: Open
Regression Test: `scripts/artifact-gate.sh` (byte-identical artifacts) + `scripts/test-accept.sh` + `tools/math-kernels/runtime_ulp.py` re-measurement for every item that touches a kernel body

The eight `builder_*math*.rs` / `builder_numeric.rs` / `builder_pow.rs` files
(~10,000 lines) that emit MFBASIC's `Float`, `Fixed`, and `Money` arithmetic carry
a large amount of hand-copied structure: the array-kernel driver prologue is
written out eight times, four scalar-vs-branchless kernel pairs differ in one
callee or one selection mechanism, two extracted helpers are re-inlined at the
site whose bodies they were extracted *from*, and several helpers/constants live
in files whose module docs describe something else. Nothing here is a
miscompile — every item is a readability, drift-surface, and reviewability
problem. The correct end state is that each duplicated shape has exactly one
emitter, **and the emitted instruction stream for every affected program is
byte-for-byte unchanged**.

**This is the highest-risk cleanup in the entire review, and it must be treated
that way.** These are numerical kernels validated to ≤1 ULP against a committed
macOS-libm oracle. `src/docs/spec/architecture/18_math-kernels.md:1-9` states that
**no platform math library is ever linked or called** — every `Float`/`Fixed`
result comes from a hand-written in-tree kernel — a property enforced by the
`no_libm_math_imports` regression test (`src/target/macos_aarch64/plan.rs:830`,
`src/target/linux_aarch64/plan.rs:434`, `src/target/linux_riscv64/plan.rs:430`).
Spec §18 further contracts that the scalar and array overloads of each function
**share one kernel**, so `math::f(x)` and `math::f([x])[0]` are bit-identical, and
that a `Float` result is bit-identical across macOS / Linux-glibc / Linux-musl.
Several of the duplicate pairs below exist *specifically* to be bit-identical and
say so in their doc comments. Unifying any of them is safe **only** if the emitted
instruction stream is provably unchanged; a "harmless" reordering that moves one
`fadd` changes a last-ULP result and silently breaks the contract. Byte-identical
artifact output is therefore the acceptance criterion for **every** item.

References:

- `src/docs/spec/architecture/18_math-kernels.md` — accuracy/determinism contract,
  the "no platform math library" rule, and the shared-kernel guarantee.
- `tools/math-kernels/README.md` — the offline ULP tooling (`capture.sh`,
  `capture_ref.c`, `gen_coeffs.py`, `runtime_ulp.py`, `ulp.py`) and the committed
  reference vectors at `tests/_data/math_kernel_ref/*.ref`.
- `planning/old-plans/plan-01-libm-kernels.md`, `plan-01-simd` §4.6 — the kernels'
  origin and validation plan.
- bug-61 / bug-74 / bug-129 / bug-137.1 — prior fixes whose comments are among the
  verbatim-copied text below.
- Found during the 2026-07 cleanup review, Agent 03 (numeric/SIMD builders).
- Converges with bug-322 (arena-alloc boilerplate) and Agent 01 finding #2: the
  hand-written `RelocIntent::Call` relocation push in E1 is one instance of a
  ~127-site repo-wide pattern.

## Current State

Measured on the worktree at `b12213d2`. File sizes:

```
$ wc -l src/target/shared/code/builder_{simd_math,simd_float_math,pow,simd_fixed_math,fixed_math,money_math,numeric,math}.rs
   1002 builder_simd_math.rs
   2372 builder_simd_float_math.rs
    927 builder_pow.rs
    387 builder_simd_fixed_math.rs
   1034 builder_fixed_math.rs
    389 builder_money_math.rs
   2025 builder_numeric.rs
   1430 builder_math.rs
```

Eight hand-written `SIMD_ALLOC_LIST_SYMBOL` call-and-relocation blocks:

```
$ grep -n "SIMD_ALLOC_LIST_SYMBOL" src/target/shared/code/builder_*.rs
builder_simd_math.rs:146,149      builder_simd_math.rs:605,608      builder_simd_math.rs:825,828
builder_simd_float_math.rs:340,343   builder_simd_float_math.rs:2154,2157
builder_simd_fixed_math.rs:42,45     builder_simd_fixed_math.rs:283,286
builder_pow.rs:411,414
```

Eight `strip_prefix("List OF ")` argument-shape blocks in one file:

```
$ grep -n 'strip_prefix("List OF ")' src/target/shared/code/builder_math.rs
91: 137: 344: 371: 406: 443: 571: 623:
```

One verbatim duplicated pair — the odd-integer test in `emit_pow_scalar`
(`builder_pow.rs:283-295`) against the identical sequence inside `emit_pow_yisint`
(`builder_pow.rs:542-555`). The two differ only in the branch target on the
non-integer arm (`&zero_ret` vs. `ret_nan`):

```rust
// builder_pow.rs:283-295, inlined inside emit_pow_scalar
self.emit(abi::float_move_x_from_d(xs, s.y));
self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs));        // y
self.emit(abi::float_convert_to_signed_x(xt, abi::FP_SCRATCH[0]));  // trunc(y)
self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], xt));
self.emit(abi::float_move_x_from_d(xm, abi::FP_SCRATCH[1]));
self.emit(abi::compare_registers(xm, xs));
self.emit(abi::branch_ne(&zero_ret));                               // non-integer -> no flip
self.emit(abi::move_immediate(xm, "Integer", "1"));
self.emit(abi::and_registers(xt, xt, xm));
self.emit(abi::compare_immediate(xt, "0"));
self.emit(abi::branch_eq(&zero_ret));                               // even -> no flip

// builder_pow.rs:542-555, the body of the extracted emit_pow_yisint
self.emit(abi::float_move_x_from_d(xs, y));
self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs));        // y
self.emit(abi::float_convert_to_signed_x(xt, abi::FP_SCRATCH[0]));  // trunc(y) (i64)
self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], xt));  // (double)trunc
self.emit(abi::float_move_x_from_d(xm, abi::FP_SCRATCH[1]));
self.emit(abi::compare_registers(xm, xs));
self.emit(abi::branch_ne(ret_nan));                                 // non-integer -> NaN
self.emit(abi::move_immediate(xm, "Integer", "1"));
self.emit(abi::and_registers(xt, xt, xm));
self.emit(abi::compare_immediate(xt, "0"));
self.emit(abi::branch_eq(&done));                                   // even
```

Everything below was individually re-verified against the source; corrections to
the review's original claims are called out inline as **CORRECTION**.

## Items

Grouped by theme. Each entry is independently landable.

### Group A — array-kernel driver duplication

#### A1 — the array-kernel driver prologue is written out eight times (~300 lines), including eight hand-repeated `RelocIntent::Call` pushes

- Sites (all verified): `builder_simd_math.rs:120` (`lower_simd_unary`), `:569`
  (`lower_simd_binary`), `:776` (`lower_simd_clamp`);
  `builder_simd_float_math.rs:318` (`lower_simd_float_unary`), `:2124`
  (`lower_simd_float_binary`); `builder_simd_fixed_math.rs:21`
  (`lower_simd_sqrt_fixed`), `:261` (`lower_simd_log_fixed`); `builder_pow.rs:382`
  (`lower_pow_array`).
- Every one of the eight emits the same ~18-line block: stage `count` into
  `ARG[0]`, the element type code into `ARG[1]`, `branch_link(SIMD_ALLOC_LIST_SYMBOL)`,
  then a hand-built `CodeRelocation { kind: RelocIntent::Call, binding: "internal",
  library: None }` push, then `reset_temporary_registers`, capture `result_base`,
  compare `RET[1]` against 0, branch to a per-site `*_alloc_ok` label, surface the
  arena tag via `move_register(return_register(), RET[1])` and
  `emit_allocation_error_return()`. The **only** differences across the eight are
  the label string and the type-code expression (`result_type_code.to_string()` vs.
  `COLLECTION_TYPE_FLOAT` vs. `COLLECTION_TYPE_FIXED`).
- Two further shapes repeat around it: a unary preamble (spill in-ptr + count to
  stack slots, reload after the call) at 5 of the 8 sites, and a binary preamble
  (load both slots, `COLLECTION_OFFSET_COUNT` on each, length-equality check →
  `emit_invalid_argument_return`) at 3 of the 8
  (`builder_simd_math.rs:580-599`, `builder_simd_float_math.rs:2138-2151`,
  `builder_pow.rs:392-403`).
- The 2-lane chunk loop is also identical modulo label prefix and the kernel-body
  callback — compare `builder_simd_math.rs:189-202` with
  `builder_simd_float_math.rs:374-387` (13 lines: `cmp pairs,0` / `b.eq done` /
  `vector_load v0` / *kernel* / `vector_store v0` / two `add_immediate …,16` /
  `subtract_immediate pairs,1` / `b loop`).
- Fix: `emit_alloc_result_list(count, type_code, label_prefix) -> result_base` plus
  `emit_two_lane_stream(…, body: impl FnMut(&mut Self))`. **Label names must be
  parameterized**, because `.ncode`/`.mir` goldens contain label strings (see
  Non-goals).
- Note the relocation push itself is the ~127-site repo-wide pattern bug-322 /
  Agent 01 #2 tracks; if a shared `emit_internal_call` lands there first, A1
  should consume it rather than growing a ninth copy of the literal.

### Group B — kernel helpers that exist but are not called

#### B1 — `emit_sin_cos_body` re-inlines `emit_cos_r_into` / `emit_sin_r_into`

- `builder_simd_float_math.rs:1183-1218` (inside `emit_sin_cos_body`, `:1175`)
  reproduces, instruction for instruction, the bodies of `emit_cos_r_into`
  (`:1331-1344`) and `emit_sin_r_into` (`:1349-1373`), with the destination
  register substituted (`&k.v23` / `&k.v24` in place of the helpers' `dst`).
- The helpers were extracted for the **scalar** path only: `emit_sin_cos_body_scalar`
  (`:1278`) calls them at `:1307`, `:1309`, `:1314`, `:1316`. The array body was
  never converted.
- Both helper doc comments promise exactly the invariant the duplication endangers:
  `:1330` — "The exact instruction sequence emit_sin_cos_body uses, so the result is
  bit-identical"; `:1347-1348` — "exactly as emit_sin_cos_body — bit-identical". A
  coefficient or reduction change must therefore be made in two places to keep
  `math::sin(x)` and `math::sin([x])[0]` agreeing, which is the spec §18
  shared-kernel guarantee.
- Fix: replace `:1183-1218` with `self.emit_cos_r_into(&k.v23, k)` and
  `self.emit_sin_r_into(&k.v24, k)`. This is the single highest-value, lowest-risk
  item in the document: the substitution is mechanical and the artifact gate proves
  it exactly.

#### B2 — `emit_tan_body` and `emit_tan_body_scalar` share 54 verbatim leading lines

- `builder_simd_float_math.rs:1383-1436` vs. `:1555-1608`. Diffed after normalizing
  the function name: the **only** differences in those 54 lines are two comment
  wordings ("→ stash in v25/v26" vs. "→ v25/v26"). They diverge for real at
  `:1437` / `:1609`, where the branchless version builds quadrant masks and the
  scalar version extracts bit0 and branches.
- The shared prefix is the double-double `sin_r`/`cos_r` computation — the same
  math as B1's helpers but retaining the `lo` halves rather than collapsing them.
- Fix: extract `emit_tan_sincos_dd(k)` covering `:1383-1436`, call it from both.

### Group C — scalar-vs-branchless kernel pairs

#### C1 — `emit_asin_acos_body` vs. `emit_asin_acos_body_scalar` differ in one callee

- `builder_simd_float_math.rs:608-661` vs. `:667-717`. Verified: byte-for-byte the
  same emit sequence (domain mask, the asin `1 - x*x` / `fsqrt` / `fdiv` reduction,
  the acos `(1-x)/(1+x)` / `fsqrt` reduction, the `2*atan` doubling), differing
  **only** in `emit_atan_core(k)` vs. `emit_atan_core_scalar(k)` and in dropped
  trailing comments.
- The `_scalar` doc (`:663-666`) already asserts the result is "bit-identical".
- Fix: one `emit_asin_acos_body(want_acos: bool, scalar: bool, k)` that selects the
  atan callee. ~50 lines removed, no instruction reordering.

#### C2 — `emit_atan_core` vs. `emit_atan_core_scalar` — **CORRECTION: much smaller than claimed**

- The review claimed "all five segment reductions duplicated, ~130 lines each". The
  measured position: `emit_atan_core` is `:739-871` (133 lines),
  `emit_atan_core_scalar` is `:976-1104` (129 lines), and the **polynomial tail is
  already factored out** — both end by calling `emit_atan_poly_recombine`
  (`:870` and `:1103`; the helper is at `:879` and its doc at `:872-878` explicitly
  says it was "Factored out so the branchless … and scalar-branching … segment
  selects run the *identical* polynomial — bit-for-bit").
- What actually duplicates is only the per-segment **reduction arithmetic** (3–5 FP
  ops per segment, 4 segments), and even that is written to a different destination
  register (`VEC_SCRATCH[3]` branchless vs. `VEC_SCRATCH[2]` scalar) because the
  selection mechanism genuinely differs: cumulative `fcmge` masks + `emit_vsel`
  versus top-down compare-and-branch to `atan_scalar_seg{0..3}` labels. The two
  functions are **not** the same instruction stream and cannot be merged without a
  parameterized emitter.
- Revised value: ~20 duplicated `emit` calls, high risk. **Recommend deferring
  C2** — extract at most `emit_atan_segment_reduce(segment, dst)` for the four
  arithmetic snippets, or leave as-is and add a cross-reference comment.

#### C3 — Money banker's tie-break implemented twice, contradicting the module doc

- `builder_money_math.rs:1-12` module doc, line 8: "so the two modes are
  implemented exactly once."
- `emit_apply_rounding` (`:30`) implements the tie-break at `:75-84`: load
  `ARENA_ROUNDING_MODE_OFFSET`, `cmp 0`, `b.eq round_up` (Commercial), else
  `and parity, quotient, 1` / `cmp 0` / `b.eq keep` (Banker → even).
- `emit_round_double_to_money_raw` (`:322`) re-implements the same decision at
  `:361-376` in the FP domain.
- **CORRECTION on scale**: this is ~10 lines, not a large duplicate; the two
  functions reach the tie by different means (integer `|rem|` vs. `half` compare
  versus an `fcmp` against 0.5). The genuine defect is the module doc's false
  "exactly once" claim.
- Fix: either extract `emit_banker_tie_branch(quotient, round_away, keep)` for the
  shared 9 lines, or (cheaper and honest) correct the module doc to say the
  *policy* is stated once and the FP conversion path carries its own copy of the
  parity test.

### Group D — repeated float classification and argument-shape decoding

#### D1 — three exponent-decode float classifiers, two near-verbatim — **CORRECTION: three, not five**

- The review listed five sites. Verified: two of them (`builder_math.rs:1270`
  `emit_float_result_check`, GPR magnitude-compare; `:1321`
  `emit_float_result_check_fp`, `fabs` + `fcmp`) are a **deliberate, documented
  GPR/FP twin pair** (plan-16 Piece B; the `_fp` doc at `:1315-1330` explains why
  both exist and asserts the emitted errors are byte-identical). They are not a
  cleanup target.
- The real family is three sites that all open with the same exponent decode
  (`shift_right_immediate(exp, bits, 52)` / `move_immediate(mask, "2047")` /
  `and_registers` / `compare_immediate(exp, "2047")`):
  - `builder_math.rs:943` `emit_float_rounding_integer_range_check` (:944-979)
  - `builder_simd_math.rs:528` `emit_float_to_int_overflow_to_err` (:530-563)
  - `builder_money_math.rs:300` `emit_float_finite_or_invalid` (:301-315)
- The first two are the near-verbatim pair, and the copy **says so**:
  `builder_simd_math.rs:524-527` — "Mirrors the terminal
  `emit_float_rounding_integer_range_check`: a value overflows when its biased
  exponent exceeds 1086, equals 2047 (Inf/NaN), or equals 1086 and is not exactly
  `-2^63`." They have already micro-drifted: the terminal version routes the
  `exp == 1086` edge through two labels (`edge` → `edge_negative` → `overflow`,
  `builder_math.rs:964-974`) while the copy folds it into a single `branch_ne`
  (`builder_simd_math.rs:552-560`). Same semantics, different instruction count.
- Fix: extract `emit_float_exponent_classify(bits) -> (exponent_reg, mask_reg)` for
  the shared 4-instruction preamble; the divergent tails stay per-site. Note the
  drift means unifying the *bodies* would change one of the two instruction
  streams — do not attempt that without a golden regeneration and an explicit
  decision on which form wins.

#### D2 — eight `strip_prefix("List OF ")` blocks with three drifting error wordings — **CORRECTION: eight, not six**

- `builder_math.rs:91`, `:137`, `:344`, `:371`, `:406`, `:443`, `:571`, `:623`
  (~70 lines total).
- Three distinct error wordings have drifted apart:
  - six sites: `"math.{fn} array overload requires a list, got {}", input.type_`
  - `:571-572`: `"math.{function} array overload requires a list"` (no `got`)
  - `:623-624`: `"math.clamp array overload requires a list"` (no `got`, function
    name hardcoded rather than interpolated)
- Fix: one `list_element_type(&self, input_type: &str, function: &str) ->
  Result<String, String>` helper with the `, got {}` wording. This is a
  compile-time error-message change only, invisible to emitted code — but confirm
  no `tests/syntax/` golden pins the two short wordings before landing.

### Group E — Fixed/integer power and CORDIC duplication

#### E1 — `emit_cordic_vectoring` vs. `emit_cordic_rotation` — the same 30-line loop

- `builder_fixed_math.rs:220-248` vs. `:363-391`. Identical structure: allocate
  `sx`/`sy`/`konst`, loop `0..CORDIC_ITERATIONS`, `i == 0` move / else
  `arithmetic_shift_right_immediate(.., i)`, `emit_const_i64(&konst,
  cordic_atan_raw(i))`, `compare_immediate(<driver>, "0")`, `branch_lt(&negative)`,
  three register updates, `branch(&done)`, three inverted updates, `label(&done)`.
- Differences: the compare driver (`vy` vs. `z`), the label prefix
  (`cordic_vec_*` vs. `cordic_rot_*`), and the add/subtract polarity of the three
  updates (vectoring drives `vy`→0 accumulating into `z`; rotation drives `z`→0).
- Fix: one `emit_cordic(mode: CordicMode, a, b, z)` with the polarity and the
  label prefix taken from `mode`. **The label prefix must be preserved per mode**
  or `.ncode` goldens churn.

#### E2 — `emit_fixed_pow` vs. the integer branch of `emit_fixed_pow_general`, with VERBATIM-copied bug-61/bug-74 comments

- `builder_numeric.rs:1626-1689` (`emit_fixed_pow`, reached from the `Fixed ^`
  operator at `builder_numeric.rs:1128`) vs. `builder_fixed_math.rs:790-847` (the
  integer branch of `emit_fixed_pow_general`, reached from `math::pow(Fixed, Fixed)`
  at `builder_math.rs:1247`).
- Both carry the same two multi-line comment blocks, copied word for word apart
  from `exponent` vs. `|exponent|` and `dst` vs. `result`:
  "Bounded-base fast path (bug-61): |base| == 1.0 has bounded powers, so the loop's
  only exit (the multiply overflow trap) never fires and it would iterate the full
  exponent. Resolve ±1.0 in closed form." and "Compare against ±1.0 through
  registers, not `compare_immediate`: the raw Fixed constants are `±2^32`, which
  exceed the x86 CMP imm32 field and fail to encode (bug-74)."
- **HAZARD — the two are NOT semantically identical.** `emit_fixed_pow`
  (`builder_numeric.rs:1636-1639`, `:1644-1648`) rejects a negative exponent and a
  non-whole exponent with `emit_invalid_argument_return`; `emit_fixed_pow_general`
  handles a negative exponent via a reciprocal tail (`builder_fixed_math.rs:848+`)
  and a fractional exponent via `exp(y*ln x)`. A naive merge changes `Fixed ^`
  semantics. Any unification must keep the guard at the caller and share only the
  ±1.0 fast path + multiply loop.
- `emit_integer_pow` (`builder_numeric.rs:1416-1472`) is a third instance of the
  same skeleton, but its bounded-base set is `{-1, 0, 1}` selected by ordered
  compares and its multiply is `emit_checked_integer_multiply` — same shape,
  materially different body. Treat it as a documentation cross-reference target,
  not a merge target.
- Fix: extract `emit_fixed_integer_power(result, base, count)` covering the ±1.0
  closed form + the truncate-to-zero early exit + the multiply loop; leave both
  callers' domain guards where they are.

### Group F — file placement and module-doc drift

#### F1 — `builder_numeric.rs` holds a 174-line String comparator and a 186-line musl `fmod` port; its `pow` twin gets its own file

- `builder_numeric.rs` has **no `//!` module doc at all** (the file opens with
  `use super::*;` at `:1`), and is 2,025 lines.
- `lower_string_comparison_binary` (`:735-810`) and `lower_string_ordering_binary`
  (`:811-908`) — 174 lines of String comparison — belong next to
  `builder_strings.rs`.
- `emit_float_fmod` (`:1839-2024`, 186 lines) is a standalone port of musl's 64-bit
  `fmod` (its doc at `:1832-1838` says so). Its direct analogue, the fdlibm `pow`
  port, **does** get its own file (`builder_pow.rs`, 927 lines) — an unmotivated
  asymmetry, and `fmod` is one of the kernels spec §18 names by file
  (`18_math-kernels.md:16-17` anchors it to `builder_numeric.rs:emit_float_fmod`).
- Fix: move the two String comparators to `builder_strings.rs`; move `emit_float_fmod`
  to a new `builder_fmod.rs`; add a `//!` doc to whatever `builder_numeric.rs`
  becomes; **update the spec anchor at `18_math-kernels.md:16-17`.**

#### F2 — `emit_money_rounding_to_integer` lives in `builder_math.rs` with one caller

- `builder_math.rs:887-941`, called only from `builder_math.rs:873`.
- `builder_money_math.rs:1-12` documents itself as the home for Money rounding.
- Fix: move to `builder_money_math.rs`, keep it `pub(super)`.

#### F3 — `builder_vector_inline.rs`'s module doc does not mention the register-native carrier it owns

- Module doc `:1-17` describes only the plan-01-vector expression-tree inlining of
  `vector::` ops.
- The file also defines `VECTOR_NATIVE_MARKER` (`:21-25`) and the whole
  register-native carrier API at `:132-221` — `is_vector_native`,
  `vector_native_lanes`, `make_vector_native`, and `materialize_value` (`:216`),
  whose own doc says "Every site that stores a value as 8 bytes or passes it as an
  argument routes through here." That is a codegen-wide escape-boundary hook, not
  an inlining detail.
- Fix: either split the carrier into its own module or extend the module doc to
  cover both concerns. Doc-only fix is acceptable and costs nothing.

#### F4 — three doc comments attached to the wrong item, one describing an algorithm that no longer exists

- `builder_simd_float_math.rs:101-107`: a paragraph describing the *GPR pinned to
  the pool base* (`math_pool_base`) sits immediately above
  `math_const_pool_words()` (`:112`) — separated only by a blank line, so it
  attaches to that function along with the correct doc at `:109-111`.
- `builder_simd_float_math.rs:719-723`: a paragraph describing an `atan(x)` core
  that evaluates "`ax*P(ax^2)` for `|x|<=1`, `pi/2 - inv*P(inv^2)` for `|x|>1`"
  and notes it is only "Faithfully rounded" — **this algorithm is no longer
  implemented.** The current `emit_atan_core` is the fdlibm 5-segment reduction
  documented at `:732-738` ("Strict <=1 ULP"). Worse, the stale paragraph is
  attached to `emit_vsel` (`:728`), a one-line `vector_bit` wrapper. A reader
  looking up atan's accuracy finds a stale "faithfully rounded" claim on the wrong
  function.
- `builder_simd_float_math.rs:2035-2036`: "Load the constant pool base address into
  the kernel's pool-base register before any broadcast). adrp+add to the read-only
  pool data symbol." — an orphaned fragment (note the unbalanced `)`) attached to
  `math_pool_base_reg` (`:2041`), which has its own correct doc at `:2037-2040`.
- Minor: the surviving atan doc's own summary line says "fdlibm **4**-segment" at
  `:732` while its next line says "one of **5** segments" at `:733`.
- Fix: delete the stale atan paragraph outright, move the pool-base paragraph to
  `math_pool_base`/`math_pool_base_reg`, delete the orphaned fragment, reconcile
  4-vs-5.

#### F5 — `numeric_element_type_code` breaks the file's grouping

- `builder_math.rs:529-536`, a tiny type-name→`COLLECTION_TYPE_*` mapper, sits
  between `lower_math_abs`'s tail and `lower_math_min_max_array` (`:539`), 450
  lines away from its natural neighbour `is_list_argument` (`:79`). Its two callers
  are `:574` and `:626`.
- Fix: move it adjacent to `is_list_argument`.

### Group G — small items

#### G1 — `emit_pow_select`'s unused `_xt` parameter is fed by three call sites

- `builder_pow.rs:844` takes `_xt: &str` and never uses it; callers at `:631`,
  `:702`, `:709` all pass `xt`.
- Fix: drop the parameter and the three arguments. Zero codegen effect.

#### G2 — `math_const_pool_offset` rebuilds an O(n²)-deduped pool on every constant broadcast

- `builder_simd_float_math.rs:184-189` calls `math_const_pool_words()` and does a
  linear `position()` per lookup. `math_const_pool_words()` (`:112-181`) builds a
  fresh `Vec<u64>` and dedupes with `words.contains(&bits)` — quadratic in the pool
  size — on every call. Called from `broadcast_f64` (`:2087`) and the integer
  broadcast (`:2103`), i.e. once per constant per kernel per function.
- Compile-time only; there is no runtime effect and no correctness issue.
- **DO NOT FIX BEFORE PHASE 1. See the Fix Design hazard note below** — this is
  the one item in the document that can silently miscompile every transcendental.

#### G3 — INT64_MIN and float mask bit patterns re-spelled inline — **CORRECTION: named constants exist for only one of them**

- `9223372036854775808` (i64::MIN / the f64 sign bit) already has **three**
  competing names, none shared: `INT64_MIN_UNSIGNED` (`builder_simd_math.rs:5`),
  `SIGN_BIT` (`builder_pow.rs:57`), and — for an unrelated purpose —
  `THREAD_RECEIVE_BLOCK_SENTINEL` (`runtime_helpers.rs:40`). It is additionally
  re-spelled bare at `builder_money_math.rs:205`, `builder_math.rs:488`,
  `builder_numeric.rs:1385`, `:1408`, `:1861`.
- For the other three patterns **no named constant exists anywhere**:
  - `4503599627370495` (2^52−1 mantissa mask) — 9 occurrences across
    `float_format.rs:71`, `builder_numeric.rs:1867`, `builder_math.rs:971`,
    `builder_simd_math.rs:555`, `builder_conversions.rs:148`, `:1232`,
    `link_thunk.rs:840`, `:1150`, `:1985`.
  - `"2047"` (biased-exponent mask) — 5 files.
  - `9218868437227405312` (+Inf bits) — `builder_math.rs:1336`,
    `builder_pow.rs:264`, `link_thunk.rs:836`, `:1146`, `:1981`.
  - `18437736874454810624` (+Inf<<1) — `builder_math.rs:1293`, one site.
- Fix: promote one shared set (`F64_SIGN_BIT`, `F64_MANTISSA_MASK`,
  `F64_EXPONENT_MASK`, `F64_POSITIVE_INF_BITS`) and retire the three INT64_MIN
  aliases into one, keeping `THREAD_RECEIVE_BLOCK_SENTINEL` separate (it is the
  same bit pattern with an unrelated meaning — deliberately not shared).

#### G4 — half the Q32.32 constants are baked, half recomputed — **CORRECTION: the remaining six are NOT host-dependent**

- Baked (bug-137.1): `CORDIC_ATAN_TABLE` (`builder_fixed_math.rs:956-987`) and
  `cordic_gain_inverse()` (`:1017`). Their docs (`:947-955`, `:1010-1016`) record
  why: the former `atan()`/`sqrt()`-based computations ran on the **build host's
  libm**, and a ≤1-ulp difference flipped `fixed_raw`'s `.round()`, producing
  byte-different binaries per build machine.
- Still computed at build time: `fixed_pi` (`:997`), `fixed_pi_over_2` (`:1002`),
  `fixed_two_over_pi` (`:1007`), `fixed_ln2` (`:1023`), `fixed_inv_ln2` (`:1028`),
  `fixed_inv_ln10` (`:1033`).
- **The review's premise does not hold.** Every one of the six feeds
  `fixed_raw(value)` = `(value * 4_294_967_296.0).round() as i64` with an argument
  built only from `std::f64::consts::*` and IEEE division (`1.0 / LN_2`,
  `1.0 / LN_10`). Multiplication by 2^32, `.round()`, and `as i64` are all exactly
  specified IEEE-754 / Rust operations with no libm involvement, so these are
  bit-reproducible on any host. There is **no build-reproducibility gap here**.
- Revised value: consistency/uniformity only. Fix if desired (bake all eight and
  delete `fixed_raw`), but do not describe it as a reproducibility fix, and do not
  prioritize it.

#### G5 — `builder_bits.rs` repeats the unary Integer-argument check four times

- `builder_bits.rs:96-99` (`lower_bits_not`), `:193-196` (`lower_bits_count_zeros`),
  `:221-224` (`lower_bits_popcount`), `:288-291` (`lower_bits_bswap`). Four
  identical `lower_value` → `type_ != "Integer"` → `Err(format!("bits.{fn} does not
  accept {}", …))` blocks, ~16 lines total.
- Fix: `lower_bits_one_integer(function, arg) -> Result<ValueResult, String>`.
  Front-end error path only, no codegen effect. Lowest value in the document.

## Goal

- Each duplicated emitter shape above has exactly one definition, and the
  `.ast`/`.ir`/`.hex`/`.nir`/`.nplan`/`.nobj`/`.ncode`/`.mir` artifacts for every
  test in `tests/` are **byte-identical** to the pre-change goldens.
- Every module doc, item doc, and spec anchor touched describes what the code
  actually does (F1's spec anchor, F3's module doc, F4's three comments, C3's
  "exactly once" claim).
- `math_const_pool_words()`'s layout is pinned by a test before anything derived
  from it is memoized or reordered.
- Measured ULP against `tests/_data/math_kernel_ref/*.ref` is unchanged for every
  function whose kernel body is touched.

### Non-goals (must NOT change)

- **Any emitted instruction.** Every item is a source-level refactor. If an item
  cannot be landed with a zero-diff artifact gate, it must be dropped or
  re-scoped — not landed with a golden regeneration "because the math is
  equivalent."
- **Label names in emitted plans.** `scripts/artifact-gate.sh` diffs `.ncode` and
  `.mir` goldens, which contain label strings. A shared emitter must take the label
  prefix as a parameter so `simd_alloc_ok` / `simd_bin_alloc_ok` /
  `pow_arr_alloc_ok` / `cordic_vec_done` / `cordic_rot_done` survive verbatim.
- **The `no_libm_math_imports` invariant.** No item may introduce a call to a
  platform math symbol, and no item may weaken or skip that test.
- **The scalar/array bit-identity guarantee** (spec §18): B1 and C1 exist to
  *strengthen* it; nothing here may create a new place where the two paths can
  diverge.
- **`Fixed ^` semantics** (E2): `emit_fixed_pow` rejects negative and fractional
  exponents; `emit_fixed_pow_general` accepts both. Do not merge the guards.
- The `.mfp` wire format, collection ABI offsets, and the Q32.32 raw
  representation.
- **Tempting wrong fix, named and forbidden**: regenerating the math goldens to
  "absorb" a refactor's instruction-stream shift. The goldens are the only thing
  standing between this cleanup and a silent last-ULP regression across every
  transcendental. If the gate is red, the refactor is wrong.
- **Second tempting wrong fix**: merging C2's two atan cores by picking whichever
  selection mechanism is shorter. They are two different instruction streams for
  two different call contexts (vectorized lanes vs. one scalar element) and both
  are load-bearing.

## Blast Radius

Actual searches, with a verdict per class of site:

- The 8 `SIMD_ALLOC_LIST_SYMBOL` sites (A1) — fixed by this bug. Enumerated above
  from `grep -n "SIMD_ALLOC_LIST_SYMBOL" src/target/shared/code/*.rs`; the ninth
  and tenth hits (`error_constants.rs:633`, `entry_and_arena.rs:1447,1450`) are the
  symbol definition and the helper's own emission — unaffected.
- The ~127 repo-wide `RelocIntent::Call` pushes (bug-322 / Agent 01 #2) — latent,
  same hazard, **out of scope**: A1 should consume a shared `emit_internal_call`
  if bug-322 lands first, but must not attempt the repo-wide sweep itself.
- The ~44 repo-wide `strip_prefix("List OF ")` sites — only the 8 in
  `builder_math.rs` are in scope (D2). The rest live in `ir/`, `monomorph/`,
  `resolver/`, `syntaxcheck/`, `binary_repr/` and `target/shared/` and answer a
  different question (type-model recursion, not builtin argument validation) —
  unaffected.
- `link_thunk.rs:836,840,1146,1150,1981,1985` (G3) — carries the same +Inf and
  mantissa-mask literals. In scope for the *constant promotion* only; its emitters
  are not otherwise touched.
- `float_format.rs:71`, `builder_conversions.rs:148,1232` (G3) — same, constants
  only.
- `builder_math.rs:1270` / `:1321` (D1) — the GPR/FP twin pair. **Explicitly out of
  scope**: documented, deliberate, and doing different work.
- `emit_integer_pow` (`builder_numeric.rs:1416`) (E2) — same skeleton, different
  bounded-base set and a different multiply helper. Out of scope for merging;
  in scope for a cross-reference comment only.
- `THREAD_RECEIVE_BLOCK_SENTINEL` (`runtime_helpers.rs:40`) (G3) — same bit
  pattern, unrelated meaning. Unaffected; must stay a separate name.
- `tools/math-kernels/*` and `tests/_data/math_kernel_ref/*.ref` — not modified by
  any item; they are the *validation instrument* for this bug.
- `src/docs/spec/architecture/18_math-kernels.md:16-17` — the `emit_float_fmod`
  anchor moves with F1. Note independently that `:23-26` names only two backends
  for the `no_libm_math_imports` test while three exist
  (`linux_riscv64/plan.rs:430`) — that is a separate spec-drift item, out of scope
  here, recorded so it is not lost.

## Fix Design

Every item is a pure source refactor whose success criterion is a zero-diff
artifact gate. The shape is: extract the shared emitter, parameterize whatever the
call sites differ in (label prefix, type code, callee, register), replace the call
sites, run `scripts/artifact-gate.sh`, and revert the item if the gate is not
clean. Items are independent; land them one commit each so a red gate localizes to
one item.

**The G2 hazard — read before touching `math_const_pool_words`.** The kernel
constant pool is **order-dependent and positional**: a constant's byte offset is
its index in the returned `Vec` times 16. Three things derive from that one order:

- `math_const_pool_offset` (`builder_simd_float_math.rs:184`) → the immediate
  offsets baked into every `broadcast_f64` / integer broadcast (`:2087`, `:2103`),
  i.e. into the emitted **code**;
- `math_const_pool_data_value` (`:193`) → the hex bytes of the emitted **data
  object**;
- `mod.rs:1257-1265` → the data object's declared `size` (`words.len() * 16`).

The dedupe is `words.contains(&bits)` over a `Vec` built in source order, so
inserting, removing, or reordering **any** entry — including a coefficient array —
shifts every subsequent offset. **There is no test anywhere that pins this
layout** (verified: the only references to the three functions are the two code
call sites and `mod.rs`).

That makes the "obvious" `OnceLock` memoization of G2 dangerous in a specific way:
if the pool is memoized behind one derivation (say `math_const_pool_offset`) while
another (`math_const_pool_data_value` or `mod.rs`'s `words.len()`) still recomputes
— or if a future change mutates the coefficient tables between the two calls — the
code's offsets and the data blob disagree and **every transcendental reads the
wrong constant**. That failure is silent: the binary links, runs, and returns
plausible-looking wrong floats. Hence Phase 1 below.

Rejected alternatives, recorded so they are not re-litigated:

- *Merge the two atan cores behind a `branchless: bool`* — rejected (C2). They are
  genuinely different instruction streams; the shared part is already extracted
  (`emit_atan_poly_recombine`).
- *Merge `emit_fixed_pow` into `emit_fixed_pow_general`* — rejected (E2). Different
  accepted domains; only the inner fast-path + loop is shareable.
- *Regenerate math goldens after a refactor* — rejected, see Non-goals.
- *Do G4's constant baking as a reproducibility fix* — rejected; the premise does
  not hold (the six remaining are IEEE-deterministic).
- *Sweep the ~127 repo-wide relocation pushes here* — rejected; that is bug-322.

## Phases

### Phase 1 — pin the pool layout and the ULP baseline (no behavior change)

- [ ] Add a unit test asserting `math_const_pool_words()` is exactly the expected
      `Vec<u64>` (full literal list) **and** that
      `math_const_pool_data_value().len() == math_const_pool_words().len() * 32`
      (two lanes × 8 bytes × 2 hex chars), **and** that
      `math_const_pool_offset(w) == i * 16` for every `(i, w)`. This is the test
      that makes G2 — and any future coefficient edit — safe. Confirm it passes
      today.
- [ ] Capture the pre-change artifact baseline: `cargo build && bash
      scripts/artifact-gate.sh ./target/debug/mfb` must report `0 diff(s)` on a
      clean tree.
- [ ] Capture the pre-change ULP baseline for every function
      `tools/math-kernels/runtime_ulp.py` can drive: `atan2, tan, pow, fmod, asin,
      acos, exp, log, log10`. Record the reported max-ULP-vs-truth and
      max-ULP-vs-macOS per function in this file. **Note**: `sin`, `cos`, and
      scalar `atan` have **no** `runtime_ulp.py` driver (verified against its
      `choices=[…]` list at `runtime_ulp.py:307-308`), so B1 / B2 / C2 have no
      runtime ULP proof available — for those items the artifact gate's
      byte-identity IS the proof, and they must not be landed with any diff.
- [ ] Confirm `no_libm_math_imports` passes on all three backends
      (`macos_aarch64/plan.rs:830`, `linux_aarch64/plan.rs:434`,
      `linux_riscv64/plan.rs:430`).

Acceptance: the pool-layout test passes; the artifact gate is clean; the ULP
baseline table is written into this file.
Commit: —

### Phase 2 — zero-risk items (no kernel body touched)

Land in this order, one commit each, artifact gate green after every one.

- [ ] G1 — drop `emit_pow_select`'s `_xt` and the three arguments.
- [ ] F4 — fix the three misattached doc comments; delete the stale 2-branch atan
      paragraph; reconcile "4-segment"/"5 segments".
- [ ] F5 — move `numeric_element_type_code` next to `is_list_argument`.
- [ ] F3 — extend `builder_vector_inline.rs`'s module doc to cover the
      register-native carrier (or split the file).
- [ ] C3 — correct `builder_money_math.rs`'s "implemented exactly once" claim
      (and optionally extract the 9-line tie branch).
- [ ] D2 — one `list_element_type` helper for the eight `strip_prefix` blocks;
      standardize on the `, got {}` wording.
- [ ] G5 — `lower_bits_one_integer` for the four `builder_bits.rs` checks.
- [ ] G3 — promote the shared f64 bit-pattern constants; unify the INT64_MIN
      aliases; leave `THREAD_RECEIVE_BLOCK_SENTINEL` alone.
- [ ] G2 — memoize the pool **only now that Phase 1's test exists**, and memoize it
      at the `math_const_pool_words()` level so all three derivations share one
      `OnceLock`. Never memoize a single derivation.

Acceptance: artifact gate reports `0 diff(s)` after each commit; the Phase 1
pool-layout test still passes.
Commit: —

### Phase 3 — kernel-adjacent refactors (byte-identity is the gate)

- [ ] B1 — replace `emit_sin_cos_body`'s inlined bodies with
      `emit_cos_r_into` / `emit_sin_r_into`. Highest value, most mechanical.
- [ ] C1 — merge `emit_asin_acos_body` / `_scalar` behind a `scalar: bool`.
- [ ] B2 — extract `emit_tan_sincos_dd` from the 54 shared leading lines.
- [ ] D1 — extract the shared 4-instruction exponent decode from the three
      classifiers; leave the divergent tails alone.
- [ ] E1 — one `emit_cordic` with a `CordicMode`, label prefix parameterized.
- [ ] E2 — extract `emit_fixed_integer_power`; leave both callers' domain guards
      in place.
- [ ] A1 — `emit_alloc_result_list` + `emit_two_lane_stream`; label prefix
      parameterized; consume bug-322's `emit_internal_call` if it has landed.
- [ ] F1 — move the String comparators to `builder_strings.rs`, `emit_float_fmod`
      to a new `builder_fmod.rs`; add a module doc to `builder_numeric.rs`; update
      the spec anchor at `18_math-kernels.md:16-17`.
- [ ] F2 — move `emit_money_rounding_to_integer` to `builder_money_math.rs`.
- [ ] C2 — **deferred by default.** Land only if it can be done with a zero-diff
      gate; otherwise close it with a cross-reference comment between the two atan
      cores.
- [ ] G4 — optional uniformity: bake the remaining six Q32.32 constants. Explicitly
      NOT a reproducibility fix.

Acceptance: after every commit, `scripts/artifact-gate.sh` reports `0 diff(s)`.
Any item that cannot achieve that is reverted, not accommodated.
Commit: —

### Phase 4 — full validation

- [ ] Full acceptance suite: `bash scripts/test-accept.sh`.
- [ ] Re-run `scripts/artifact-gate.sh` on the final tree: `0 diff(s)`.
- [ ] **Re-measure ULP** for every `runtime_ulp.py`-drivable function and diff
      against the Phase 1 baseline: identical numbers required, not merely
      "still ≤1 ULP".
- [ ] `no_libm_math_imports` green on all three backends.
- [ ] Confirm the `git diff --stat` for `tests/**/golden/` is **empty**.

Acceptance: full suite green; zero golden churn; ULP table identical to Phase 1.
Commit: —

## Validation Plan

- **Regression test**: the Phase 1 `math_const_pool_words` layout test — the one
  new test this bug adds, and the guard that makes G2 and every future coefficient
  edit safe.
- **Primary gate**: `scripts/artifact-gate.sh ./target/debug/mfb` must report
  `0 diff(s)` after **every** commit. It diffs `.ast`, `.ir`, `.hex`, `.nir`,
  `.nplan`, `.nobj`, `.ncode`, and `.mir` goldens across every `tests/` project, so
  it catches both instruction-stream and label-name changes.
- **Runtime proof (required for every item touching a kernel body — B1, B2, C1,
  C2, D1, E1, E2, A1)**: `python3 tools/math-kernels/runtime_ulp.py <fn>` for each
  of `atan2 tan pow fmod asin acos exp log log10`, against
  `tests/_data/math_kernel_ref/`. This compiles and runs a real MFBASIC program, so
  it measures the machine code actually emitted — golden equality alone is **not**
  sufficient acceptance for these items. Report must be identical to the Phase 1
  baseline. For `sin`/`cos`/scalar `atan` no driver exists; byte-identity is the
  only available proof and is therefore mandatory.
- **Invariant**: `no_libm_math_imports` on `macos_aarch64`, `linux_aarch64`, and
  `linux_riscv64`.
- **Full suite**: `bash scripts/test-accept.sh`.
- **Doc sync**: `src/docs/spec/architecture/18_math-kernels.md:16-17` (the
  `emit_float_fmod` anchor, F1). The stale two-backend claim at `:23-26` is
  recorded here but belongs to a separate spec-drift item.

## Open Decisions

- **C2 (atan cores)** — recommend **defer / document** over extracting
  `emit_atan_segment_reduce`. The measured duplication is ~20 emit calls, the two
  selection mechanisms are genuinely different, and the polynomial tail is already
  shared. (§Group C)
- **C3 (banker tie-break)** — recommend **fix the module doc** over extracting the
  9-line branch; the doc claim is the actual defect. (§C3)
- **G4 (Q32.32 constants)** — recommend **leave as-is** or bake purely for
  uniformity; the reproducibility premise does not hold. (§G4)
- **A1 vs. bug-322 ordering** — recommend landing bug-322's `emit_internal_call`
  first so A1 consumes it rather than introducing a competing helper. (§Blast
  Radius)
- **D1 (drifted classifier pair)** — the two near-verbatim classifiers already emit
  different instruction counts for the `exp == 1086` edge. Recommend sharing only
  the 4-instruction preamble and leaving the drift, rather than picking a winner
  and regenerating goldens. (§D1)

## Summary

Nineteen verified cleanup items across ~10,000 lines of numeric/SIMD codegen. The
engineering risk is **not** in the refactors themselves — most are mechanical —
but in the fact that these are ≤1-ULP-validated kernels whose duplicate pairs
exist specifically to be bit-identical, with no unit tests underneath them and
only golden artifacts and an offline ULP harness as evidence. The single most
dangerous item is G2: the constant pool is order-dependent, three things derive
from that order, and nothing pins the layout — which is why Phase 1 adds that test
before anything else happens. Four of the review's original claims were measured
down (C2's atan duplication is much smaller than reported, D1 is three sites not
five, C3 is ten lines not a large duplicate, and G4's build-reproducibility premise
is false); those corrections are recorded inline. The highest value for the least
risk is B1 — one function call replacing 36 inlined lines, restoring the
`math::sin(x)` ≡ `math::sin([x])[0]` invariant that spec §18 contracts and that the
helpers' own doc comments already promise. Untouched: every emitted instruction,
every label name, the Q32.32 representation, `Fixed ^` semantics, and the
no-platform-libm invariant.
