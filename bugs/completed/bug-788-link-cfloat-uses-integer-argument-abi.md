# bug-788: LINK CFloat arguments do not use the native float ABI

Last updated: 2026-10-03
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness
Status: Fixed
Regression Test: `tests/runtime/rt_link_cfloat.rs` (C oracle, every boundary); `float_precision_conversions_encode` in each of `src/arch/{aarch64,x86_64,riscv64}/encode/tests.rs`

A LINK wrapper accepts `CFloat`, but silently passes the argument as raw binary64
bits in an integer register. The correct behavior is to convert the MFBASIC
binary64 value to C binary32 and pass it in the target C ABI's float location.
The same native call declared with `CDouble` works in the reproduction.

## Failing Reproduction

Run on macOS AArch64 from the repository root:

```sh
mkdir -p /tmp/mfb-cfloat/src /tmp/mfb-cfloat/vendor
cat > /tmp/mfb-cfloat/project.json <<'EOF'
{"name": "native", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "entry": "main", "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}], "targets": ["native"], "libraries": {"probe": [{"os": "macos", "type": "vendor", "source": "libprobe.dylib"}]}}
EOF
cat > /tmp/mfb-cfloat/src/main.mfb <<'EOF'
IMPORT io
LINK "probe" AS probe
  FUNC check(x AS Float) AS Integer
    SYMBOL "check_float"
    ABI (x CFloat) AS value CInt32
    RETURN value
  END FUNC
  FUNC checkDouble(x AS Float) AS Integer
    SYMBOL "check_double"
    ABI (x CDouble) AS value CInt32
    RETURN value
  END FUNC
END LINK
FUNC main() AS Nothing
  io::print(toString(probe::check(1.5)))
  io::print(toString(probe::checkDouble(1.5)))
END FUNC
EOF
cat > /tmp/mfb-cfloat/probe.c <<'EOF'
int check_float(float x) { return x == 1.5f; }
int check_double(double x) { return x == 1.5; }
EOF
cc -dynamiclib /tmp/mfb-cfloat/probe.c -o /tmp/mfb-cfloat/vendor/libprobe.dylib
./target/release/mfb build /tmp/mfb-cfloat
/tmp/mfb-cfloat/build/native.out
```

Observed: `0` then `1`, exit 0. Expected: `1` then `1`.
Measured with the same inputs under `/tmp/mfb-language-audit/native` using
`./target/release/mfb build /tmp/mfb-language-audit/native` and
`/tmp/mfb-language-audit/native/build/native.out`.
The build's uncovered-platform warnings are unrelated to the host result.
Other targets have not been executed.

## Root Cause

`src/codegen/link/thunk/link_thunk.rs:lower_link_thunk` special-cases only
`CAbiType::Double` when assigning floating argument registers; `CFloat` falls
through to the integer path, without binary64-to-binary32 conversion.
`src/ir/link.rs:abi_ctype_valid_as_argument` accepts the spelling, so the wrong
ABI reaches runtime without a diagnostic.

## Goal and Non-goals

Support faithful CFloat argument and result marshaling across supported native
ABIs, including CSTRUCT fields and OUT slots where accepted. Preserve CDouble,
integer, resource, buffer, and package serialization behavior. Do not disguise
the failure by changing the binding to CDouble: that changes the C signature.

## Blast Radius

Search: `rg -n 'CAbiType::Float|CAbiType::Double|fn marshal_struct' src/codegen/link/thunk/link_thunk.rs`.

- `lower_link_thunk`: direct arguments reproduced; return and OUT conversion
  share the same scalar marshaling and must be audited and covered in this fix.
- `marshal_struct_in` / `marshal_struct_out`: accepted CFloat fields need
  binary32 conversion; audit and cover in this fix (not runtime-tested here).
- CDouble direct arguments: unaffected contrast, prints `1`.
- Source and compiled bindings share the thunk; both paths need regression tests.

## Fix Design

Use explicit conversions and the target's external floating-point calling
convention. Separate a scalar float argument from a pointer to an OUT float.
Confirm field width and rounding and finite-value handling at each boundary.

## Phases

### Phase 1 — failing tests and audit

- [x] Add the native C oracle and a failing runtime test for this reproduction.
- [x] Audit and add cases for returns, OUT slots, and CSTRUCT fields.
- [x] Verify source and compiled-package paths on supported targets.

Acceptance: failures demonstrate ABI/value mismatches, with CDouble controls.
Commit: —

### Phase 2 — fix

- [x] Implement binary32 conversion and correct ABI placement at all affected sites.
- [x] Update the language native-libraries specification to the verified behavior.

Acceptance: the oracle cases pass without changing native C signatures.
Commit: —

### Phase 3 — full validation

- [x] Run the full suite before any proven-needed golden changes.
- [x] Run `cargo test --release` and the repository artifact/runtime gates.
- [x] Execute the oracle on all supported native targets.

Acceptance: full gates pass and platform results agree with the C oracle.
Commit: —

## Validation Plan

Runtime execution of the C oracle is the primary proof; compilation is insufficient.
Keep existing tests/goldens unchanged unless independently proved wrong. The
language spec describes the existing raw-bit limitation until the fix lands.

## Resolution (2026-10-03)

**New backend ops.** `fcvt_s_from_d` / `fcvt_d_from_s` (`CodeOp::FCvtSFromD` /
`FCvtDFromS`) are wired through the op table, MIR, peephole and operand lists.
They encode as AArch64 `fcvt Sd,Dn` / `fcvt Dd,Sn`, x86-64 `cvtsd2ss` /
`cvtss2sd`, and RISC-V `fcvt.s.d` (RNE) / `fcvt.d.s`.

**Thunk changes, one per boundary.**

- Argument / `BIND IN` field from a parameter: rounded to binary32. A finite
  value too large for binary32 raises `ErrFloatOverflow` (`77050015`). The C
  slot holds NaN-boxed binary32 bits, which RISC-V requires.
- Return, OUT slot, struct field: widened exactly (4-byte load for OUT and
  fields), then the `CDouble` finiteness gate applies (`ErrFloatNaN` /
  `ErrFloatInf`).
- `CONST` pins and `BIND IN` integer literals on `CFloat`/`CDouble` store the
  numeric value, not the integer's raw bits.

**Siblings found and fixed in the same change.**

1. **OUT `CDouble` crash.** An OUT `CDouble` (and now OUT `CFloat`) slot staged
   its word's *address* into an FP register. Main crashes with SIGSEGV at
   `out_double(double, double*)` (probe row `outd`). Pointers now always use
   integer registers.
2. **Win64 argument positions.** Win64 assigns argument registers by position,
   shared across banks, but the thunk counted int and FP slots separately. On
   main, `_jn(int, double)` read its double from the wrong xmm register
   (`jn2 BAD` on the Windows box). `packages/sqlite3`'s
   `sqlite3_bind_double(stmt, i, double)` had the same shape. New
   `RegisterModel::external_arguments_are_positional` (Win64 answers TRUE).

**Runtime proof, fix vs main.** Both oracles are the same programs on every
target:

- **C probe:** `tests/runtime/rt_link_cfloat.rs` / `/tmp/b788`.
- **System libm:** `/tmp/b788m` (`sqrtf`, `modff`, `scalbnf`, `ldexp`, `jn`/`_jn`).

| target | oracle | fix | main |
|---|---|---|---|
| macOS AArch64 (host) | C probe + libm | all rows correct | wrong values, SIGSEGV |
| Linux AArch64 glibc (box 2226) | libm | all correct | `sqrtf 0.00`, SIGSEGV |
| Linux RISC-V musl (box 2229) | C probe | all correct | wrong values, SIGSEGV |
| Windows x86-64 (box 206, real hardware) | libm (`ucrtbase`) | all correct | wrong `CFloat`s, `jn2 BAD` |
| Linux x86-64 SysV | — | **not run** (boxes 2227/2228 down) | — |

The x86-64 SysV path uses the encodings proven on Windows and the per-bank
counting proven on AArch64 and RISC-V. It still needs one run when a Linux
x86-64 box is up. Spec: `mfb spec language native-libraries` (the "raw 64-bit"
deviation note is replaced by the contract) and the AArch64, x86-64, RISC-V and
MIR instruction-set topics.
