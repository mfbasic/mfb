# bug-270: emitted-binary exploit-mitigation LOW cluster — no BTI/PAC note, no stack canaries, macOS no hardened runtime

Last updated: 2026-07-17
Effort: large (3h–1d across items)
Severity: LOW
Class: Security (defense-in-depth)

Status: Assessed — all three items consciously deferred (rationale below); no safe
unconditional change is available today. No regression risk introduced.
Regression Test: (none — no code change)

## Resolution (assessment)

Each item was investigated (LNK-10 empirically). All three are LOW defense-in-depth
mitigations that are either infeasible as a straight change, gated on
infrastructure that does not exist, or feature-scale codegen; none is a broken
guarantee. They are recorded here as tracked hardening features with implementation
sketches, matching the repo's low-severity-cluster convention.

- **LNK-10 — `__DATA_CONST` maxprot=R: INFEASIBLE as described.** The `__got`
  (`S_NON_LAZY_SYMBOL_POINTERS`) is bound by dyld during load, so `__DATA_CONST`
  must be writable while fixups are applied — `maxprot` must be ≥ `initprot` (RW).
  Setting `maxprot=R` would forbid the load-time GOT binding and fail the image.
  The actual read-only-after-fixup protection is already provided by
  `SG_READ_ONLY` (bug-187); the residual "an attacker who already has code
  execution + a way to call `mprotect` could re-elevate" is defense-in-depth of a
  defense-in-depth and cannot be closed without a different (non-dyld) GOT-binding
  scheme.
- **LNK-10 — CS_RUNTIME: DEFERRED (no notarization flow; local-run/​LINK risk).**
  Setting the hardened-runtime flag (`0x10000`, making CodeDirectory flags
  `0x30002`) was tested: the hand-rolled ad-hoc signature still validates
  (`codesign -v`) and the binary runs locally. But the finding scopes it to
  *distributed* builds, and mfb has no Apple notarization/distribution flow —
  ad-hoc + hardened-runtime is not notarizable, so it buys nothing for
  distribution, while risking `dlopen` of user-vendored native LINK libraries
  under hardened-runtime library validation. mfb's own `signing_metadata`
  (`.mfb_sign`) is a package-provenance marker, not an Apple team identity, so it
  is not a valid gate. Deferred until a real macOS distribution/notarization path
  exists to gate it. The non-goal ("CS_RUNTIME gating should not break local
  runs") is honored by leaving local builds at `0x20002`.
- **LNK-05 — BTI/PAC note + landing pads: DEFERRED (feature-scale codegen).**
  Emitting `.note.gnu.property` with `AARCH64_FEATURE_1_BTI` is safe only in
  lockstep with a `BTI c` landing pad at *every* indirect-branch target (function
  entries, import-stub targets) — without them the note makes valid indirect
  branches fault. That is a cross-cutting change to every aarch64 prologue, HIGH
  regression risk for a LOW mitigation. Scoped as a dedicated codegen feature.
- **LNK-09 — stack canaries: DEFERRED (feature-scale codegen).** Per-frame canary
  emission (load TLS `__stack_chk_guard`, store in frame, verify before return,
  branch to `__stack_chk_fail`) for frames with a fixed-size stack buffer is a
  codegen feature. LOW: the model relies on the bounds-checked collection/string
  runtime, so there is no demonstrated overflowable generated stack buffer.

### Recommendation

If pursued, LNK-05 and LNK-09 warrant their own plan-NN feature docs (they are
codegen features, not bug fixes); LNK-10 CS_RUNTIME warrants a macOS
distribution/notarization design first. None should be landed as an unconditional
change against the current hardware-validated macOS/Linux output.

Three individually-LOW exploit-mitigation gaps in the emitted-binary hardening
surface from audit-2 that lack their own bug docs. Each is a mitigation the
generated code/linker does not (yet) provide; none is a disabled default that the
platform would otherwise supply for free — the compiler must emit them itself.
Grouped per the repo's low-severity-batch convention; the higher-severity linker
items are bug-186/187/224/225/263.

References:

- `planning/audit-2-linker-hardening.md` (LNK-05, LNK-09, LNK-10).

## Findings

### LNK-05 — no `.note.gnu.property` / BTI / PAC; no landing pads at indirect-branch targets
- Location: `src/os/linux/link/mod.rs:459-467` (`emit_import_stub`); no
  `GNU_PROPERTY_AARCH64_FEATURE_1_BTI`/PAC note generated on any target (grep: zero
  hits).
- Symptom: aarch64 binaries opt out of BTI (branch-target-identification) and PAC,
  so indirect-branch targets have no landing-pad enforcement — a JOP/ROP hardening
  gap. Only meaningful once the toolchain also emits `BTI c` landing pads.
- Fix: emit `.note.gnu.property` with `AARCH64_FEATURE_1_BTI` and place `BTI c`
  at indirect-branch targets (function entries, stub targets). Larger change;
  scope carefully.

### LNK-09 — no stack-smashing protection anywhere in emitted code (informational)
- Location: no canary/`__stack_chk_*` emission (grep: zero hits); frames are
  generated directly by `src/arch/*/encode`.
- Symptom: a stack buffer overflow in a generated frame is undetected at return.
  The platform will not auto-insert canaries, so this is a design gap, not a
  disabled default; LOW absent a demonstrated overflowable generated stack buffer
  (the model relies on bounds-checked collection/string runtime).
- Fix (if pursued): emit a per-frame canary (load from TLS `__stack_chk_guard`,
  store in the frame, verify before return, branch to `__stack_chk_fail`) for
  frames that contain a fixed-size stack buffer.

### LNK-10 — macOS ad-hoc signature omits hardened runtime; `__DATA_CONST` maxprot stays RW
- Location: `src/os/macos/link/commands.rs:522-523` (code-directory flags
  `0x20002 = CS_ADHOC|CS_LINKER_SIGNED`; `CS_RUNTIME 0x10000` not set);
  `commands.rs:62` (`__DATA_CONST` `maxprot=0x3`).
- Symptom: no hardened runtime, and `__DATA_CONST` `maxprot=RW` permits a runtime
  `mprotect` back to writable despite `SG_READ_ONLY` — weakening the bug-187
  read-only-GOT protection. Ad-hoc signing is normal for local builds; matters for
  distributed builds.
- Fix (distributed builds): set `CS_RUNTIME` and lower `__DATA_CONST` `maxprot` to
  R so the read-only region cannot be re-elevated.

## Goal

- aarch64 output carries a BTI property note + landing pads (LNK-05); generated
  frames with fixed stack buffers carry a canary (LNK-09); macOS distributed
  builds set `CS_RUNTIME` and a read-only `__DATA_CONST` maxprot (LNK-10).

### Non-goals (must NOT change)

- Runtime behavior / ABI of correctly-behaving programs.
- Local-build ad-hoc signing ergonomics (CS_RUNTIME gating should not break
  `mfb build` local runs).
