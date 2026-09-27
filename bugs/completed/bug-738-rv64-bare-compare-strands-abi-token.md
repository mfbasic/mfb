# bug-738: riscv64 build of any mouse-aware stdin program fails with `unknown rv64 integer register '%retMFB0'`

Last updated: 2026-09-26
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Closed
Regression Test: `src/arch/riscv64/select.rs:tests::bare_compare_on_abi_tokens_realizes_them`,
`src/target/linux_riscv64/mod.rs:tests::mouse_aware_stdin_reads_encode`

`mfb build --target linux-riscv64` refuses every console program that uses the
terminal mouse (`term::enableMouse`, `term::pollMouse`, …) together with a stdin read
(`io::readChar`, `io::readLine`, `io::pollInput`). The build dies in the encoder with
`error: unknown rv64 integer register '%retMFB0'` and writes no binary.
`examples/swat/tui` is the in-tree victim (it is one of the `linux-riscv64` failures
of `scripts/build-examples.sh`); every other target builds it.

## STATUS: FIXED (564a6e18a)

Shipped as designed: `select_riscv64`'s bare-compare arms keep register operands
typed (`field_operand`, `snapshot_lhs`, `store_flag_rhs(&Operand)`). No deviation.
Verified on merged tip 1ca1f73a1: `scripts/artifact-gate.sh target/release/mfb all`
→ 0 diffs over 2160 goldens (no golden moved, as predicted); full `cargo test
--no-fail-fast` → all 244 test binaries `ok`. Runtime: both riscv64 boxes (2229, 2232)
refused ssh, so the proof ran under qemu-user riscv64 containers (Alpine musl, Debian
trixie glibc): a mouse+`pollInput`/`readChar`/`readLine` probe matched the host output
byte for byte, and `examples/swat/tui` rendered, took a mouse click and quit cleanly
on both libcs.

**The single correct behavior:** the rv64 selector's bare-compare snapshot realizes a
convention-token operand (`%retMFB0` → `a0`) exactly as every other rv64 operand is
realized, so these programs build for `linux-riscv64` and run correctly there.

References:

- `mfb spec architecture riscv64-instruction-set` — "Flagless control flow" and "ABI
  register remapping".
- `.ai/arch-abi.md` "riscv64 flag-emulation reserved slots" (bug-381, the rhs snapshot).
- `.ai/codegen-invariants.md` "Registers are typed values, never text (bug-721)".
- Found while triaging lane C's `scripts/build-examples.sh` failures (present at
  0cd2db799, when the example was `examples/bugs/tui`).

## Failing Reproduction

```
mfb build --target linux-riscv64 examples/swat/tui
```

Minimal (`src/main.mfb` of any executable project):

```
IMPORT io
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  LET c AS String = io::readChar()
  RETURN 0
END FUNC
```

- Observed: `error: unknown rv64 integer register '%retMFB0'`; no executable.
- Expected: `Wrote executable to build/p-glibc.out` / `build/p-musl.out`.

`-ncode --target linux-riscv64` still writes the plan, and it shows the stranded
token in `runtime.io.readChar` (and `runtime.io.pollInput`):

```
{"op": "label", "name": "_mfb_rt_io_io_readChar_read_retry_cont"}
{"op": "mov", "dst": "gp", "src": "%retMFB0"}
```

Contrast cases (work today): the same program without the mouse call (the pump is
not emitted, and every other `cmp` fuses with an adjacent branch); every other target
(the flag machines keep `cmp` as-is; there is no snapshot); an `io::readLine`/`readChar`
program on rv64 without `term` mouse use.

| Program | linux-riscv64 | other 4 targets |
| --- | --- | --- |
| mouse + `io::readChar` | fails ✗ | builds ✓ |
| mouse + `io::readLine` | fails ✗ | builds ✓ |
| mouse + `io::pollInput` | fails ✗ | builds ✓ |
| `io::readChar` alone | builds ✓ | builds ✓ |

## Root Cause

`src/codegen/io/stdin/stdin_broadcast.rs:emit_stdin_next_byte` ends with
`cmp_imm abi::return_register(), 0` after its `_cont` label, leaving the flags for
the caller's later branch. In the mouse pump that branch is not adjacent, so on rv64
the compare stays **bare** and `src/arch/riscv64/select.rs:select_riscv64` takes its
flag-emulation path: it snapshots the lhs into `gp` (and, for `cmp`, spills the rhs to
the per-thread word through `store_flag_rhs`).

That path read the operands with `field_value`, which **renders** each `Operand` to a
`String`, and rebuilt the `mov gp, …` / `str_u64 …` from that text. A typed convention
token (`Operand::Abi`, rendered `%retMFB0`) thus became a raw text operand.
`remap_riscv_abi` realizes only the typed `Operand::Abi` form (plan-85-A) and the
legacy role tokens `realize_abi_token` knows; `%retMFB0` is neither, so it passed
through to `src/arch/riscv64/encode/operand.rs`, which rejects it.

The fused path (`expand_fused`) was immune: plan-85-D gave it `get_op`, which keeps
the typed operand, with a comment stating exactly this rule. The bare path was missed.

## Goal

- The two regression tests pass; `mfb build --target linux-riscv64 examples/swat/tui`
  writes both executables; the binary runs on a riscv64 box.

### Non-goals (must NOT change)

- No emitted byte of any program that already built: for a vreg, a physical name, or
  a legacy token the typed and rendered operand realize to the same register, so
  every existing `linux-riscv64` golden must stay byte-identical.
- Not "fixing" `emit_stdin_next_byte` to emit a physical `x0` or to re-fuse the
  compare: the token is correct there (bug-387 made the MFB result explicit), and the
  selector must handle a bare compare on any operand.
- Not teaching `realize_abi_token` the text spelling of the convention tokens: that
  re-opens the string-register door bug-721 closed.

## Blast Radius

Search: every `render()` / `field_value` in `src/arch/riscv64/select.rs` and
`src/arch/riscv64/v128.rs`, plus a `linux-riscv64` build of every executable example.

- `select_riscv64` `CmpImm` lhs → gp — **fixed** (the observed failure).
- `select_riscv64` `Cmp` lhs → gp and rhs → `store_flag_rhs` — **fixed** (same hazard,
  not observed: no emitter today leaves a bare register-register compare on a token).
- `select_riscv64` `CmpImm` rhs, standalone-branch `target` — unaffected: an immediate
  and a label, never a register.
- `expand_fused` `get` — unaffected: used only for immediates (`rhs` of `cmp_imm`) and
  the `cond` text; registers go through `get_op`.
- `select_riscv64` `AddrOf` — unaffected: keeps `dst` typed; renders only `symbol`.
- `v128.rs` `f()` renders (`base`/`dst`/`src` of the scalarized loads/stores and lane
  moves) — latent, same shape, NOT observed: no emitter feeds a v128 op a convention
  token today. Measured, not assumed: a pre-fix `-ncode --target linux-riscv64` sweep of
  all 877 projects under `tests/rt-behavior`, `tests/byte-identity`, `tests/rt-error`
  and `examples/` for a literal `"%…"` register operand found exactly one site — this
  bug's (`examples/swat/tui`). A string literal `"%zz"` (the `value` of an encoding
  fixture's data) is not a register. Left as-is: changing it would move no byte and
  has no failing input to prove against.

## Fix Design

Read the bare compare's register operands with a typed `field_operand` (the
`get_op` rule) and build the `gp` snapshot (`snapshot_lhs`) and the rhs spill
(`store_flag_rhs(&Operand)`) with `physical_field`, so `remap_riscv_abi` realizes
the token. Rejected: realizing tokens in `field_value` (a second realizer, and the
same trap waits for the next caller); making `remap_riscv_abi` parse `%retMFB…` text
(bug-721's forbidden string door).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] `bare_compare_on_abi_tokens_realizes_them` (selector unit: `cmp_imm %retMFB0`,
      `cmp %retC1, %argMFB2`, each with a non-adjacent branch) — RED: `%retMFB0`,
      `%retC1`, `%argMFB2` survive selection.
- [x] `mouse_aware_stdin_reads_encode` (whole program: mouse + `readChar`/`readLine`/
      `pollInput`, lowered, relaxed and encoded for linux-riscv64) — RED:
      `unknown rv64 integer register '%retMFB0'`.
- [x] Blast-radius audit above.

Acceptance: both tests fail for the documented reason.
Commit: 10fa559e9 (doc), 564a6e18a (tests landed with the fix; RED recorded in its message)

### Phase 2 — the fix

- [x] `src/arch/riscv64/select.rs`: `field_operand`, `snapshot_lhs`,
      `store_flag_rhs(&Operand)`; the `Cmp`/`CmpImm` arms keep register operands typed.
- [x] Spec: `architecture/16_riscv64-instruction-set.md` bare-compare bullet (it also
      still described the pre-bug-381 "rhs kept by name" design).

Acceptance: both tests pass; the example builds.
Commit: 564a6e18a; 535b873eb (`.ai/codegen-invariants.md` lesson)

### Phase 3 — regenerate expected outputs + full validation

- [x] `scripts/artifact-gate.sh target/release/mfb all` — 0 diffs / 2160 goldens (1ca1f73a1).
- [x] Full `cargo test --no-fail-fast` — 244 binaries ok (1ca1f73a1).
- [x] Run the rebuilt `examples/swat/tui` on riscv64 — boxes 2229/2232 refused ssh;
      run under qemu-user riscv64 containers instead (musl + glibc), see STATUS.

Acceptance: gate 0 diffs; full suite green; the example runs on riscv64.
Commit: no golden moved; verification only (this archive commit)

## Validation Plan

- Regression tests: the two above (`cargo test --bin mfb`).
- Runtime proof: build `examples/swat/tui` for linux-riscv64 and run it on 2229/2232.
- Doc sync: the spec bullet above.
- Full suite: `cargo test`; `scripts/artifact-gate.sh target/release/mfb all`.

## Summary

A one-seam fix: the rv64 flag-emulation path rendered a typed register to text. Risk
is confined to the bare-compare expansion, and the golden gate proves no existing
byte moves.
