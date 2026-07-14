# plan-00-A — MIR Scaffold (`-codegen mir`, `-mir` dump, AArch64 passthrough)

Last updated: 2026-06-29

The foundation of the MIR effort (`planning/mir.md`). Introduce a target-neutral machine
IR **layer** between NIR and the AArch64 backend, behind a build flag, with the layer
initially a near-1:1 mirror of today's AArch64 instruction stream. The win of this plan is
**not** neutrality yet — it is the *plumbing*: the `MirOp` type, the dual code path, the
`-mir` dump, and the **byte-identical self-diff gate** that de-risks every later plan.

Reads `planning/mir.md` (the design + resolved decisions); this is its §13 step 1.

## 1. Goal

- Define `MirOp` reusing today's `op + string-fields` instruction shape (`mir.md §10`/§12.6).
- Add `mfb build -codegen <direct|mir>` (default **`direct`** = today's no-MIR path). Under
  `-codegen mir`: NIR lowers to a **MIR** stream, then a thin **MIR→AArch64** pass produces
  the existing `CodeInstruction`/`CodeOp` stream the rest of the backend already consumes.
- Add `mfb build -mir` — the neutral counterpart to `-ncode`: the MIR stream as ASM-as-JSON
  (`format: "mfb-mir"`, virtual registers, no `target`/`arch`), reusing the `-ncode` writer.
- **Gate:** `-codegen mir` is **byte-identical** to `-codegen direct` (`.ncode`/`.nobj`/
  final binary) across the full acceptance suite. The MIR ops at this stage are allowed to
  be AArch64-shaped; only the *layer* must exist and round-trip exactly.

### Non-goals

- **No neutralization yet** (flags, addressing, immediates, SIMD, helpers stay AArch64-ish
  — those are plans B–F). No new ISA. No behavior change. `direct` stays default and intact.

## 2. Current State

`src/target/shared/code/` builders emit AArch64 `CodeOp` directly (via `abi::`); reg-alloc
+ peephole run on that; `-ncode` dumps it (flat: labels + branches, post-allocation). There
is no layer between NIR and the AArch64 stream. The `-regalloc bump|linear-scan` precedent
(plan-03) is the model for a flag-gated dual path with a byte-identical oracle.

## 3. Design

- **`MirOp`**: a neutral op enum + the same `Vec<(&str, String)>` field bag as
  `CodeInstruction`. Phase-A `MirOp` variants mirror the `CodeOp` set 1:1 (rename later).
- **Pipeline under `-codegen mir`:** `NIR → lower_to_mir() → MIR → select_aarch64(MIR) →
  CodeInstruction stream → (existing reg-alloc, peephole, encode)`. For Phase A the lowering
  is the *same logic* as today's builders, retargeted to emit `MirOp`; `select_aarch64` is
  the trivial inverse map (MirOp→CodeOp 1:1). The point is the seam, not the transformation.
- **Where reg-alloc runs (decide here, document in mir.md §2):** simplest is to keep
  allocation where it is — on the AArch64 stream *after* `select_aarch64` — so plan A is a
  pure insert with zero allocator change. (Allocating on MIR is a later option, not now.)
- **`-mir` writer:** the `-ncode` JSON serializer parameterized over the op set; emits the
  MIR stream before `select_aarch64`.

## 4. Phases

1. `MirOp` + the field bag + the `-mir` JSON writer (no pipeline change; dump an empty/
   stub MIR to prove the format).
2. `-codegen <direct|mir>` flag; under `mir`, run `lower_to_mir` (today's builder logic on
   `MirOp`) + `select_aarch64` (1:1) feeding the existing reg-alloc/peephole/encode.
3. **Byte-identical gate:** full unfiltered `scripts/test-accept.sh` produces identical
   `.ncode`/binaries under `-codegen mir` vs `direct`. A diff harness (like the regalloc
   differential) over the suite.

## 5. Validation

- The whole-suite self-diff (`direct` vs `mir`) must be **byte-identical** — this is the
  acceptance criterion and the safety net for B–G.
- `-mir` emits well-formed JSON for representative programs; `-ncode` unchanged.
- `direct` remains the default and the shipping path until plan-00-G.

## Summary

A pure structural insert: a neutral-shaped layer + a flag + a dump + a self-diff oracle,
with `direct` untouched as the default. It buys nothing for users yet — it buys the ability
to neutralize the backend (plans B–F) one op-family at a time, each provable byte-identical
against the path it replaces, on a target we already trust.
