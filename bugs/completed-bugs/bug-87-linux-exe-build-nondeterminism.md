# bug-87: Linux executables are not byte-deterministic across identical compiler runs

STATUS: OPEN (root cause not yet isolated). Pre-existing — reproduces at
`fa89792d`, before plan-34-D. Filed 2026-07-10, discovered by plan-34-D
Phase 6's baseline byte-diff validation.

## Goal (the single correct behavior)

Two invocations of the same `mfb` binary on the same project and `-target`
produce byte-identical executables. `for i in 1 2 3 4; do mfb build -target
linux-aarch64 tests/rt-behavior/math/math_package_valid && shasum
math_package_valid-glibc.out; done` prints one hash, four times.

## Failing reproduction

At `fa89792d` (pre-plan-34-D) and at HEAD, on the macOS host:

```
$ for i in 1 2 3 4; do
    rm -f tests/rt-behavior/math/math_package_valid/math_package_valid-*.out
    ./target/debug/mfb build -target linux-aarch64 tests/rt-behavior/math/math_package_valid >/dev/null
    shasum tests/rt-behavior/math/math_package_valid/math_package_valid-glibc.out
  done
99cced2acf940bc6…   ← four DIFFERENT hashes in four runs (fa89792d compiler)
f907a86c5ed57c88…
7a05ee4a890b1685…
f85bdea10c128998…
```

Observed matrix (3–4 runs each, both the `fa89792d` and plan-34-D compilers):

| program | macos-aarch64 | linux-aarch64 | linux-x86_64 | linux-riscv64 |
|---|---|---|---|---|
| parser-hello-world | stable | stable | stable | stable |
| control-flow-match | stable | stable | stable | stable |
| thread-drop-cleanup | stable | stable | stable | stable |
| tls-connect-google-rt | stable | stable | stable | stable |
| math_package_valid | stable | **flaky** | (imm32 build error) | **flaky** (overlapping) |
| math_simd_signzero_tail_valid | stable | **flaky** | **flaky** (overlapping) | (fmin_v build error) |
| datetime-instant-valid | stable | **flaky** (overlapping) | **flaky** (overlapping) | **flaky** (overlapping) |

"flaky (overlapping)" = repeated runs of the two compilers produce overlapping
hash *sets* — same distribution, low cardinality. The math tests on
linux-aarch64 have high-cardinality flake (≥4 distinct hashes).

Not runtime-affecting: all flaky binaries run correctly on the x86 (`ssh -p
2227`) and riscv (`ssh -p 2229`) boxes; only the byte layout varies.

## What is known / eliminated

- The per-package `.nobj` artifact gate is deterministic (969 goldens, 0
  diffs, many runs) — the flake is in the per-executable assembly (runtime
  helpers / data objects / link step), exactly the region bug-85 showed the
  per-pkg gate does not cover.
- Per-function `ncode` body hashing across 3+3 runs (fa89792d vs plan-34-D):
  every *function* body's hash set overlaps between compilers. A first-cut
  attribution pointed at `_mfb_math_const_pool`, but that is a red herring as
  a *root cause*: `builder_simd_float_math.rs:math_const_pool_words` builds
  the pool by fixed-order `Vec` dedup — fully deterministic — so either the
  crude symbol-splitting parser mis-attributed neighboring content, or the
  *placement* (not content) of the data object varies.
- Programs that flake pull in the float/math runtime (`datetime` formats
  floats; the math tests use the kernels). Programs that don't (hello,
  match, threads, tls) are stable everywhere.
- macOS executables of the same programs are stable — the flake needs the
  linux dual-flavor (glibc+musl in one invocation) path, or a
  linux-link-specific stage.

## Root-cause hypotheses (each with its elimination test)

1. **Data-object / helper emission order from a hash-ordered container** in
   the linux executable assembly (`target/linux_*/mod.rs` or the shared plan
   collection consumed there). Test: dump the symbol *order* of two differing
   builds (`nm`-equivalent over the produced ELF or the plan JSON) and diff —
   if the order permutes, grep that path for `HashMap`/`HashSet` iteration.
2. **The known `variants_for_union` HashMap iteration**
   ([[union-drop-codegen-nondeterminism]], pre-existing memory note) reaching
   more than union drop order — e.g. through monomorph output order feeding
   the executable's function sequence. Test: same symbol-order diff; if
   function order is stable but bodies differ, this is eliminated.
3. **GOT/import-stub ordering in the linux linker path** (the macOS twin of
   this is a known note: import-stub GOT page divergence). Test: diff the two
   builds' relocation/import tables specifically.

## Non-goals

- Do not change any *deterministic* output: the fix must be
  order-stabilization (e.g. `BTreeMap`/sorted iteration), after which ONE
  canonical output is chosen — expect a one-time golden refresh for any
  affected goldens, but no semantic change.
- Do not "fix" by loosening the validation that found it (plan-34-D's
  flake-aware exe-diff is a workaround for THIS bug, not the desired end
  state).
- Runtime behavior, ABI, and section layout semantics stay as-is.

## Blast-radius audit

- Any consumer needing reproducible builds (release artifacts, byte-diff
  validation like plan-34-D Phase 6, cache keys) — affected today.
- The `.nobj`/`-mir`/`-ncode` per-package artifacts — unaffected
  (deterministic, gate-verified).
- macOS executables — no observed flake, but audit the same container
  pattern there before declaring unaffected.

## Phased fix (test-first)

1. **Reproduce + localize (no behavior change).** Add a repeatable harness
   (script or test) that builds a flaky program twice and diffs the ELF
   section/symbol tables; record which stream permutes. Acceptance: the
   permuting container is named in this document.
2. **Stabilize the order** (sorted/`BTreeMap`/insertion-order container) at
   the named site(s). Acceptance: the Phase-1 harness passes 10/10 runs on
   the full observed matrix; full `cargo test` + artifact gate green;
   one-time golden refresh if any golden captured a now-canonicalized order.
3. **Regression guard.** Fold the double-build determinism check into the
   test suite (cheap: one small program per target, two builds, `cmp`).

Commit: —
