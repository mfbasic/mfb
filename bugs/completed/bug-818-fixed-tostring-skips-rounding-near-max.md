# bug-818: `toString(Fixed)` truncates instead of rounding near the top of the range

Last updated: 2026-10-04
Effort: small (fix); the native goldens of every Fixed `toString` move
Severity: LOW
Class: Correctness

Status: Fixed
Regression Test: `tests/rt-behavior/conversions/fixed-tostring-rounds-near-max`

Found while fixing bug-813 (bug-staging-9).

## Failing Reproduction

- `toString(2147483647.996F)` prints `2147483647.99`; correct is `2147483648.00`
  (half-away rounding at 2 places).
- At 1 decimal place, `2147483647.996F` prints `…7.9`; correct is the value
  rounded up.

## Root Cause

`builder_strings.rs:emit_fixed_to_string_value` deliberately skips its
half-away rounding step near the top of the range. Its comment calls truncating
"the safe answer". The rounding cannot overflow: 2^63 plus the rounding increment
fits in an unsigned 64-bit value. So the guard is unnecessary, and it gives wrong
digits.

## Goal

`toString` of a Fixed rounds half away from zero at every magnitude, including
next to the maximum and the minimum. No overflow, and no change for values the
current path already rounds.

## Blast Radius

The native code of every Fixed `toString`: `.ncode`/`.ncodesum` goldens move in
the batch gate.

## Phases

- [x] RED fixture: values next to ±max at 2 places and at 1.
- [x] Remove the truncation guard; round on unsigned magnitudes.
- [x] Verify on macOS plus one non-macOS box; goldens in the gate.

## Resolution

RED fixture in the commit before the fix; fix in the commit that archives this
document. `emit_fixed_to_string_value` drops the `i64::MAX - half` guard and
always adds the bias to the unsigned magnitude (at most 2^63 + 2^31, which fits;
the shifts are logical). The guard was signed, so it also truncated the
negative side next to the minimum (`-2147483647.996F` printed
`-2147483647.99`), not only the positive side. The compile-time fold
`numeric::fixed_default_to_string_text` (also the debugger's renderer) mirrored
the guard and is changed identically; its unit test gains the near-max cases.

Evidence: `toString(2147483647.996F)` 2147483647.99 → 2147483648.00, at 1 place
…47.9 → 2147483648.0, at 0 places `2147483647.5F` → 2147483648, the maximum
Fixed at 9 places → 2147483648.000000000, `-2147483647.996F` → -2147483648.00;
in-range values unchanged. Runtime and folded results agree. Verified at -O0
and -O1 on macOS arm64, 2226 (aarch64 glibc), 2229 (riscv64 musl) and 206
(Windows x86-64), byte-compared with the macOS output. `cargo test --release
--bin mfb -- numeric:: debugger declared_errors spec`: the numeric tests pass
(the one failure, `debugger::prompt::tests::show_type_needs_a_debug_build_s_arenas`,
fails identically at the parent and is fixed separately).

Golden moves (native code of every Fixed `toString`), measured by hashing
`mfb build -ncode -target <t>` against each goldened `.ncodesum` with and without
this fix: `byte-identity/general`, `byte-identity/collections`,
`byte-identity/math` and `byte-identity/vector`, each on all five targets
(linux-aarch64, linux-riscv64, linux-x86_64, macos-aarch64, windows-x86_64); the
other 137 goldened `.ncodesum` files are unchanged. No `.ir`/`build.log` golden
moves. Spec: `language builtin-functions` (toString of a Fixed).
