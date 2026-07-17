# bug-270: emitted-binary exploit-mitigation LOW cluster — no BTI/PAC note, no stack canaries, macOS no hardened runtime

Last updated: 2026-07-17
Effort: large (3h–1d across items)
Severity: LOW
Class: Security (defense-in-depth)

Status: Open
Regression Test: (none yet)

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
