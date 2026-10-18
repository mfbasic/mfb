# bug-1008: valid large constant `chunks`/`window` sizes and strides fail during native code generation

Last updated: 2026-10-07
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (compiler robustness)

Status: Open
Regression Test: `tests/runtime/rt_collection_large_constant_range_size.rs` (proposed; ensure valid huge size and stride constants do not panic in codegen)

## Cause (coordinator, bug-staging-24)

- One fix with bug-1007; see its Cause. The panic and the rejected immediates are the compile-time `size * 8` / `size`/`stride` immediates of the `chunks`/`window` fast paths, which bug-1007's run-time sizing removes; any constant still emitted is materialized through a register, not `add_immediate`.

The public collection APIs accept any positive `Integer` chunk/window size and `window` stride. When those values are large constants, the native fast paths perform unchecked host-size arithmetic or emit them as backend immediates while constructing their layouts and loops. On macOS aarch64, a valid size of `2305843009213693952` panics the compiler in debug mode for both `collections::chunks` and `collections::window`, although the equivalent dynamic-size calls build and return the expected result. A valid String `window` stride of `1000000000` is rejected by the AArch64 immediate-decomposition guard, while the equivalent dynamic call succeeds.

**Correct behavior:** compiling a program that passes a positive `Integer` size larger than the input length must not panic or reject valid source. `chunks([1, 2, 3], size)` should return one three-element chunk, and `window([1, 2, 3], size)` should return an empty list.
Likewise, a positive constant stride larger than the input length must compile and return the same result as its dynamic equivalent.

## Failing Reproduction

The first program:

```
IMPORT collections
FUNC main() AS Integer
  LET xs = collections::chunks([1, 2, 3], 2305843009213693952)
  RETURN len(xs)
END FUNC
```

The second program:

```
IMPORT collections
FUNC main() AS Integer
  LET xs = collections::window([1, 2, 3], 2305843009213693952)
  RETURN len(xs)
END FUNC
```

Build either with `target/debug/mfb build -q <project>` on macOS aarch64, compiler built at HEAD `9d2240565`:

- Observed: `chunks` panics at `src/codegen/builtins/collections/func_chunks.rs:67:53` and `window` panics at `src/codegen/builtins/collections/func_window.rs:85:57`, both with `attempt to multiply with overflow`.
- Expected: both compile; running the first returns a single chunk containing three values, and running the second returns zero windows.

Dynamic-size controls were built and run with `SIZE=2305843009213693952`: `chunks([1, 2, 3], size)` prints one chunk of length 3, and `window([1, 2, 3], size)` prints 0. A `chunks` constant of the same magnitude on the detached release build instead failed during AArch64 lowering with `add immediate ... is too wide to decompose`.

The String `chunks` size, `window` size, and `window` stride fast paths also reject large constants. Each form can be reproduced in a project with the same `project.json` used above:

```
IMPORT collections
FUNC main() AS Integer
  LET chunks = collections::chunks(["a", "b"], 1000000000)
  RETURN len(chunks)
END FUNC
```

```
IMPORT collections
FUNC main() AS Integer
  LET windows = collections::window(["a", "b"], 1000000000)
  RETURN len(windows)
END FUNC
```

```
IMPORT collections
FUNC main() AS Integer
  LET windows = collections::window(["a", "b"], 1, 1000000000)
  RETURN len(windows)
END FUNC
```

- Observed on macOS aarch64: each program makes `target/debug/mfb build /tmp/mfb_collections_audit` fail with `AArch64 add immediate 1000000000 is too wide to decompose (more than 8 chunks)`.
- Dynamic contrast: passing `stride` from `os::getEnvOr("STRIDE", "1000000000")` in the third program builds and prints `1` then `a`.

## Root Cause

`src/codegen/builtins/collections/func_chunks.rs`, `lower_collection_chunks_call`, computes `block_stride = COLLECTION_HEADER_SIZE + (size as usize) * 8` without checked arithmetic. `src/codegen/builtins/collections/func_window.rs`, `lower_collection_window_call`, similarly computes `inner_block_size = COLLECTION_HEADER_SIZE + (size as usize) * 8`. In debug compiler builds, the multiplication panics for this valid positive `i64`; in optimized builds the cast/multiplication wraps and later generated address arithmetic may be rejected. Separately, `lower_collection_window_string_call` emits `size` and `stride` through `abi::add_immediate`, so large valid constants hit the backend's immediate-decomposition error (`func_window.rs`, String fast-path loop). These dimensions exceed the input list, so the native specialization does not need to reserve or address their full declared maximum.

## Blast Radius

- Fixed-width `collections::chunks` constant fast path — reproduced compiler panic; also emits a large immediate for the requested step.
- Fixed-width `collections::window` constant fast path — reproduced compiler panic.
- String `chunks` fast path — valid large constant size rejected by immediate decomposition.
- String `window` fast path — valid large constant stride rejected by immediate decomposition; valid large constant sizes share the immediate-based construction pattern.
- Dynamic-size generic implementations — contrast paths; both large-size results above compile and run correctly.

## Fix Design

Before specializing, use checked sizing and only take a native layout fast path when all compile-time offsets and capacities are representable. Otherwise keep the valid call on the generic implementation, which already handles a size larger than the input without allocating a huge result. Avoid turning this input condition into an undocumented source restriction.

## Validation Plan

- Regression: literal-size programs compile and return the stated results in debug and release compiler builds; String-element counterparts with large size and stride constants compile and agree with dynamic controls.
- Runtime proof: execute the generated programs and inspect result lengths and contents.
- Doc sync: none expected; valid positive `Integer` sizes keep their existing API contract.
- Full suite: acceptance and runtime suites after implementation.
