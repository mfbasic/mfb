# bug-934: `math::min`/`math::max` on `Float` give a different signed zero on x86-64 than on AArch64/riscv64 (`min(-0.0, 0.0)` prints `0.00` on x86, `-0.00` elsewhere)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness (cross-target determinism)

Status: Open
Coordinator note: the x86-64 result was measured by the review agent only; the coordinator did not re-run it.
Regression Test: tests/rt-behavior/math/float-min-max-signed-zero-rt (new; name suggested — run on every backend)

`math::min(Float)` / `math::max(Float)` (scalar and `List OF Float`) lower to `fminnm_d`/`fmaxnm_d`
(and the `.2d` lane forms). AArch64 `fminnm`/`fmaxnm` and RISC-V `fmin.d`/`fmax.d` order the zeros
(`-0.0 < +0.0`), so `min` of a `±0` pair is always `-0.0` and `max` always `+0.0`. The x86-64 backend
uses `minsd`/`maxsd` (and `minpd`/`maxpd`), which on a `±0` tie return the **second** operand, so the
result depends on argument order. The program prints different text per target:
`toString(math::min(nz, z))` is `-0.00` on macOS-aarch64 and linux-riscv64, `0.00` on linux-x86_64.

`architecture x86_64-instruction-set` acknowledges the difference but claims it cannot be observed:
"they differ from AArch64 `fminnm`/`fmaxnm` only on a NaN input or a ±0 tie (neither reachable
through `math::min`/`max`)". A ±0 tie is reachable with ordinary arithmetic (`-1.0 * 0.0`), and
`toString` renders the sign. `architecture math-kernels` promises Float math results are
bit-identical across targets in practice.

**Correct behavior:** one result on every target — the AArch64/RISC-V IEEE-754-2019 `minimumNumber`/
`maximumNumber` ordering (`min(±0 pair) = -0.0`, `max(±0 pair) = +0.0`) — or, if the owner accepts the
divergence, the x86 spec sentence must stop calling it unreachable.

References:

- `src/docs/spec/architecture/15_x86_64-instruction-set.md` "Floating point and SIMD → xmm" (the
  "neither reachable" sentence).
- `src/docs/spec/architecture/18_math-kernels.md` "Determinism contract".
- `src/docs/spec/architecture/14_aarch64-instruction-set.md` (`fminnm_d`/`fmaxnm_d`),
  `16_riscv64-instruction-set.md` ("`fmin.d`/`fmax.d` back `fminnm_d`/`fmaxnm_d` … matching AArch64").
- Found in hardening batch B12 (architecture/15, cross-target differential).

## Failing Reproduction

```
IMPORT io
IMPORT math
FUNC main() AS Nothing
  LET z AS Float = 0.0
  LET nz AS Float = -1.0 * z
  io::print("min(+0,-0)=" & toString(math::min(z, nz)) & " min(-0,+0)=" & toString(math::min(nz, z)))
  io::print("max(+0,-0)=" & toString(math::max(z, nz)) & " max(-0,+0)=" & toString(math::max(nz, z)))
  FOR EACH v IN math::min([z, nz], [nz, z])
    io::print("lmin " & toString(v))
  NEXT
  FOR EACH v IN math::max([z, nz], [nz, z])
    io::print("lmax " & toString(v))
  NEXT
END FUNC
```

| Environment | Output |
| --- | --- |
| macos-aarch64 (native run) | `min(+0,-0)=-0.00 min(-0,+0)=-0.00` / `max(+0,-0)=0.00 max(-0,+0)=0.00` / `lmin -0.00` ×2 / `lmax 0.00` ×2 ✓ |
| linux-riscv64 (`--target linux-riscv64`, run under qemu in docker `alpine:3.20` riscv64) | identical to aarch64 ✓ |
| linux-x86_64 (`--target linux-x86_64`, glibc binary run in an amd64 docker container under Rosetta) | `min(+0,-0)=-0.00 min(-0,+0)=0.00` / `max(+0,-0)=-0.00 max(-0,+0)=0.00` / `lmin -0.00`, `lmin 0.00` / `lmax -0.00`, `lmax 0.00` ✗ |

(The x86 result is the architectural `MINSD`/`MAXSD` rule — "if both are zeros of either sign, the
second operand is returned" — so it is not a Rosetta artifact; no physical x86 box was reachable to
re-run it.)

## Root Cause

`src/arch/x86_64/encode/emitter.rs:sse_arith` encodes `fminnm_d`/`fmaxnm_d` as a bare
`minsd`/`maxsd` (and the packed `fmin_v`/`fmax_v`-style lane ops used by the list overloads as
`minpd`/`maxpd`), whose ±0 tie returns the source operand. AArch64 `FMINNM`/`FMAXNM` and RISC-V
`FMIN.D`/`FMAX.D` treat `-0.0` as less than `+0.0`.

## Goal

- The reproduction prints the aarch64 lines on linux-x86_64 too (scalar and list).
- Non-zero and NaN-free finite results unchanged; x86 still uses SSE (e.g. `minsd` plus an OR of the
  operands' sign bits when both compare equal for `min`, AND for `max`).

### Non-goals (must NOT change)

- AArch64 and riscv64 encodings (already agree).
- NaN behavior (MFBASIC never feeds NaN to these; out of scope).

## Blast Radius

- `fminnm_d`/`fmaxnm_d` on x86 — `math::min`/`max`/`clamp(Float)` scalar.
- `v128.fmin`/`v128.fmax` on x86 (`minpd`/`maxpd`) — `List OF Float` overloads (measured: list forms
  diverge the same way). `v128.fmin`/`fmax` are documented NaN-propagating, and any kernel that uses
  them on a ±0 pair is exposed — not surveyed.
- `math::clamp(Float)` — measured `clamp(-0.0, 0.0, 1.0)` = `-0.00` and `clamp(0.0, -0.0, -0.0)` =
  `0.00` on all three targets (no divergence observed for those two inputs).
